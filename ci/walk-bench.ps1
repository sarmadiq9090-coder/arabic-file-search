# Index-build bench for the directory walk (src/walk.rs): builds the index of
# one drive (C:\ by default) with the old walker (BAHTH_WALK=legacy) and the
# new one, from the same test exe, one build per process, and prints for
# each run the entries, the time, the walk threads, the most threads the
# process had and its peak private bytes. Not run by any workflow.
#
#   pwsh -File ci\walk-bench.ps1                                   # builds the test exe itself
#   pwsh -File ci\walk-bench.ps1 -Rounds 5 -Threads 4,8,16,32      # also a thread-count sweep
#   pwsh -File ci\walk-bench.ps1 -Modes legacy,fast,dirinfo -Out bench-out
#
# Runs alternate (legacy, fast, then fast, legacy, ...) after one warm-up
# walk, so both see the same, warm, file cache. The first build on a real PC
# is cold; to compare cold walks, reboot before each of
#   pwsh -File ci\walk-bench.ps1 -NoWarmup -SkipCompare -Rounds 1 -Modes legacy
#   pwsh -File ci\walk-bench.ps1 -NoWarmup -SkipCompare -Rounds 1 -Modes fast
# The bench is a #[ignore] test (walk::tests::walk_bench); the test exe's
# counting allocator costs both walkers alike (the old one allocates more).
param(
    [string]$TestExe = '',
    [string]$Root = 'C:\',
    [int]$Rounds = 3,
    [string[]]$Modes = @('legacy', 'fast'),
    [int[]]$Threads = @(),
    [switch]$NoWarmup,
    [switch]$SkipCompare,
    [string]$Out = ''
)
$ErrorActionPreference = 'Stop'
$Modes = @($Modes | ForEach-Object { $_ -split ',' } | Where-Object { $_ })

if (-not $TestExe) {
    Write-Host '== building the test exe (cargo test --release --no-run)'
    $json = & cargo test --release --no-run --message-format=json 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'cargo test --no-run failed' }
    foreach ($l in $json) {
        if ($l -notlike '{*') { continue }
        $m = $l | ConvertFrom-Json
        if ($m.reason -eq 'compiler-artifact' -and $m.target.name -eq 'bahth' -and $m.profile.test -and $m.executable) {
            $TestExe = $m.executable
        }
    }
    if (-not $TestExe) { throw 'no test executable for bahth in the cargo output' }
}
$TestExe = (Resolve-Path $TestExe).Path
Write-Host "test exe: $TestExe"
Write-Host "root: $Root   cores: $([Environment]::ProcessorCount)"

function Start-Test([string]$test, [string]$mode, [int]$threads) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $TestExe
    $psi.Arguments = "$test --exact --ignored --nocapture --test-threads=1"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.EnvironmentVariables['BAHTH_WALK'] = $mode
    $psi.EnvironmentVariables['BAHTH_WALK_BENCH_ROOT'] = $Root
    if ($threads -gt 0) { $psi.EnvironmentVariables['BAHTH_WALK_THREADS'] = "$threads" }
    elseif ($psi.EnvironmentVariables.ContainsKey('BAHTH_WALK_THREADS')) { $psi.EnvironmentVariables.Remove('BAHTH_WALK_THREADS') }
    $p = [Diagnostics.Process]::Start($psi)
    $stdout = $p.StandardOutput.ReadToEndAsync()
    $stderr = $p.StandardError.ReadToEndAsync()
    $peakThreads = 0
    while (-not $p.HasExited) {
        try { $p.Refresh(); $peakThreads = [Math]::Max($peakThreads, $p.Threads.Count) } catch { }
        Start-Sleep -Milliseconds 100
    }
    $p.WaitForExit()
    return [pscustomobject]@{ Exit = $p.ExitCode; Text = $stdout.Result; Err = $stderr.Result; PeakThreads = $peakThreads }
}

function Run-Build([string]$mode, [int]$threads, [string]$label) {
    $r = Start-Test 'walk::tests::walk_bench' $mode $threads
    $line = ($r.Text -split "`r?`n") | Where-Object { $_ -like 'walk-bench *' } | Select-Object -First 1
    if (-not $line) { throw "walk_bench printed no result (exit $($r.Exit)):`n$($r.Text)`n$($r.Err)" }
    $kv = @{}
    foreach ($m in [regex]::Matches($line, '(\w+)=("[^"]*"|\S+)')) { $kv[$m.Groups[1].Value] = $m.Groups[2].Value.Trim('"') }
    return [pscustomobject]@{
        Config       = $label
        Mode         = $kv['mode']
        Entries      = [int64]$kv['entries']
        Seconds      = [Math]::Round([double]$kv['ms'] / 1000, 2)
        WalkThreads  = [int]$kv['threads']
        PeakThreads  = $r.PeakThreads
        PeakPrivMB   = [int64]$kv['peak_private_mb']
        Fingerprint  = $kv['fingerprint']
        Detail       = $kv['detail']
    }
}

$configs = @()
foreach ($m in $Modes) { $configs += [pscustomobject]@{ Label = $m; Mode = $m; Threads = 0 } }
foreach ($t in $Threads) { $configs += [pscustomobject]@{ Label = "fast@$t"; Mode = 'fast'; Threads = $t } }

if (-not $NoWarmup) {
    Write-Host '== warm-up walk (not counted)'
    $w = Run-Build 'fast' 0 'warm-up'
    Write-Host ("   {0} entries in {1} s ({2})" -f $w.Entries, $w.Seconds, $w.Detail)
}

if (-not $SkipCompare) {
    Write-Host '== same entries? (legacy, then fast and dirinfo, in one process)'
    $c = Start-Test 'walk::tests::walk_compare' 'fast' 0
    ($c.Text -split "`r?`n") | Where-Object { $_ -like 'walk-compare*' -or $_ -like '  only*' } | ForEach-Object { Write-Host "   $_" }
}

$runs = @()
for ($r = 1; $r -le $Rounds; $r++) {
    $order = @($configs)
    if ($r % 2 -eq 0) { [array]::Reverse($order) }
    foreach ($cfg in $order) {
        $run = Run-Build $cfg.Mode $cfg.Threads $cfg.Label
        $run | Add-Member -NotePropertyName Round -NotePropertyValue $r
        $runs += $run
        Write-Host ("round {0} {1,-9} {2,9} entries {3,8} s  walk threads {4,3}  process threads {5,3}  peak private {6,6} MB  {7}" -f `
                $r, $cfg.Label, $run.Entries, $run.Seconds, $run.WalkThreads, $run.PeakThreads, $run.PeakPrivMB, $run.Fingerprint)
    }
}

function Median([double[]]$xs) {
    $s = $xs | Sort-Object
    if ($s.Count -eq 0) { return 0 }
    if ($s.Count % 2 -eq 1) { return $s[[int][Math]::Floor($s.Count / 2)] }
    return ($s[$s.Count / 2 - 1] + $s[$s.Count / 2]) / 2
}

$summary = foreach ($cfg in $configs) {
    $mine = @($runs | Where-Object Config -eq $cfg.Label)
    [pscustomobject]@{
        Config      = $cfg.Label
        Runs        = $mine.Count
        Entries     = [int64](Median ($mine | ForEach-Object { $_.Entries }))
        Seconds     = [Math]::Round((Median ($mine | ForEach-Object { $_.Seconds })), 2)
        WalkThreads = ($mine | Select-Object -First 1).WalkThreads
        PeakThreads = ($mine | Measure-Object PeakThreads -Maximum).Maximum
        PeakPrivMB  = [int64](Median ($mine | ForEach-Object { $_.PeakPrivMB }))
        Detail      = ($mine | Select-Object -First 1).Detail
    }
}
Write-Host ''
Write-Host '== medians'
$summary | Format-Table -AutoSize | Out-String | Write-Host

$verdict = @()
$old = $summary | Where-Object Config -eq 'legacy'
if ($old) {
    foreach ($s in $summary | Where-Object Config -ne 'legacy') {
        $faster = if ($old.Seconds -gt 0) { [Math]::Round(100 * ($old.Seconds - $s.Seconds) / $old.Seconds, 1) } else { 0 }
        $mem = if ($old.PeakPrivMB -gt 0) { [Math]::Round(100 * ($s.PeakPrivMB - $old.PeakPrivMB) / $old.PeakPrivMB, 1) } else { 0 }
        $same = @($runs | Where-Object { $_.Config -eq $s.Config -or $_.Config -eq 'legacy' } | Group-Object Round | Where-Object {
                (@($_.Group.Fingerprint | Sort-Object -Unique)).Count -gt 1 }).Count
        $verdict += [pscustomobject]@{
            Config        = $s.Config
            FasterPercent = $faster
            TargetMet     = ($faster -ge 40)
            PeakMemPct    = $mem
            MemoryOk      = ($mem -le 10)
            EntriesDelta  = $s.Entries - $old.Entries
            RoundsWithDifferentFingerprints = $same
        }
    }
    Write-Host '== against legacy (target: >= 40% faster, peak private bytes at most +10%)'
    $verdict | Format-Table -AutoSize | Out-String | Write-Host
    Write-Host 'A live drive changes a little between walks (logs, temp files): a few entries'
    Write-Host 'and a different fingerprint are expected; the compare step lists the paths.'
}

if ($Out) {
    New-Item -ItemType Directory -Force $Out | Out-Null
    [pscustomobject]@{ root = $Root; cores = [Environment]::ProcessorCount; runs = $runs; medians = $summary; verdict = $verdict } |
        ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $Out 'walk-bench.json')
    $md = @('# Walk bench', '', "Root $Root, $([Environment]::ProcessorCount) logical cores.", '',
        '| Config | Runs | Entries | Seconds (median) | Walk threads | Process threads (max) | Peak private MB |',
        '|---|---|---|---|---|---|---|')
    foreach ($s in $summary) { $md += "| $($s.Config) | $($s.Runs) | $($s.Entries) | $($s.Seconds) | $($s.WalkThreads) | $($s.PeakThreads) | $($s.PeakPrivMB) |" }
    if ($verdict) {
        $md += @('', '| Config | Faster than legacy | Peak memory vs legacy |', '|---|---|---|')
        foreach ($v in $verdict) { $md += "| $($v.Config) | $($v.FasterPercent)% | $($v.PeakMemPct)% |" }
    }
    $md | Set-Content -Encoding utf8 (Join-Path $Out 'walk-bench.md')
    Write-Host "wrote $Out\walk-bench.json and walk-bench.md"
}
