# Speed comparison on the runner's C: drive, after ci/demo.ps1 has built the
# index: InstaFind's command line (which loads the saved index from disk on
# every run) against the tools Windows itself offers for a whole-drive name
# search: `dir /s` in cmd, PowerShell's Get-ChildItem, and the Windows Search
# index when the service exists. Writes bench.md and bench.json to $Out.
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null

function Time-It([scriptblock]$block) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $count = & $block
    $sw.Stop()
    return [pscustomobject]@{ Seconds = [Math]::Round($sw.Elapsed.TotalSeconds, 2); Count = [int]$count }
}

function InstaFind([string]$q) {
    $o = Join-Path $env:RUNNER_TEMP 'bench-out.txt'
    $line = "-s `"$q`" --all --scope `"C:\`" --count"
    Start-Process -FilePath $Exe -ArgumentList $line -NoNewWindow -Wait -RedirectStandardOutput $o | Out-Null
    $t = [IO.File]::ReadAllText($o).Trim()
    if ($t -match '(\d+)') { return [int]$Matches[1] } else { return 0 }
}

function Dir-S([string]$pattern) {
    $o = Join-Path $env:RUNNER_TEMP 'bench-dir.txt'
    cmd /c "chcp 65001 >nul & dir `"C:\$pattern`" /s /b /a > `"$o`" 2>nul"
    return @(Get-Content -LiteralPath $o -Encoding utf8 -ErrorAction SilentlyContinue).Count
}

function Gci([string]$pattern) {
    return @(Get-ChildItem -LiteralPath 'C:\' -Recurse -Force -Filter $pattern -ErrorAction SilentlyContinue).Count
}

function WinSearch([string]$word) {
    $conn = New-Object -ComObject ADODB.Connection
    $conn.Open("Provider=Search.CollatorDSO;Extended Properties='Application=Windows';")
    $rs = $conn.Execute("SELECT System.ItemPathDisplay FROM SYSTEMINDEX WHERE System.FileName LIKE '%$word%'")
    $n = 0
    while (-not $rs.EOF) { $n++; $rs.MoveNext() }
    $conn.Close()
    return $n
}

$entries = (Get-Item (Join-Path $env:LOCALAPPDATA 'BahthSari\index.bin')).Length
$rows = @()
foreach ($case in @(
        @{ Query = 'report'; Pattern = '*report*'; Note = 'English word' },
        @{ Query = 'احمد'; Pattern = '*احمد*'; Note = 'Arabic, typed without hamza (the file is named أحمد)' })) {
    $q = $case.Query
    Write-Host "== $q"
    # Warm-up run so every tool starts with the same disk cache state.
    InstaFind $q | Out-Null
    $runs = 1..5 | ForEach-Object { Time-It { InstaFind $q } }
    $best = ($runs | Sort-Object Seconds)[2]   # median of five
    $rows += [pscustomobject]@{ Query = $q; Note = $case.Note; Tool = 'InstaFind (command line, loads the index each run)'; Seconds = $best.Seconds; Found = $best.Count }
    $r = Time-It { Dir-S $case.Pattern }
    $rows += [pscustomobject]@{ Query = $q; Note = $case.Note; Tool = 'cmd: dir /s'; Seconds = $r.Seconds; Found = $r.Count }
    $r = Time-It { Gci $case.Pattern }
    $rows += [pscustomobject]@{ Query = $q; Note = $case.Note; Tool = 'PowerShell: Get-ChildItem -Recurse'; Seconds = $r.Seconds; Found = $r.Count }
    try {
        $r = Time-It { WinSearch $q }
        $rows += [pscustomobject]@{ Query = $q; Note = $case.Note; Tool = 'Windows Search index'; Seconds = $r.Seconds; Found = $r.Count }
    } catch {
        $rows += [pscustomobject]@{ Query = $q; Note = $case.Note; Tool = 'Windows Search index'; Seconds = $null; Found = $null; Error = "$($_.Exception.Message)".Trim() }
    }
    $rows | Where-Object Query -eq $q | Format-Table -AutoSize | Out-String | Write-Host
}

$os = (Get-CimInstance Win32_OperatingSystem).Caption
$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
$md = @("Measured on a GitHub Actions runner: $os, $cpu, $(Get-Date -Format 'yyyy-MM-dd').", "Index file: $([Math]::Round($entries / 1MB)) MB.", '',
        '| Search | Tool | Time (s) | Found |', '|---|---|---:|---:|')
foreach ($r in $rows) {
    $t = if ($null -ne $r.Seconds) { $r.Seconds } else { 'not available' }
    $f = if ($null -ne $r.Found) { $r.Found } else { '—' }
    $md += "| ``$($r.Query)`` | $($r.Tool) | $t | $f |"
}
$md -join "`n" | Set-Content -LiteralPath (Join-Path $Out 'bench.md') -Encoding utf8
$rows | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Out 'bench.json') -Encoding utf8
Get-Content (Join-Path $Out 'bench.md') | Write-Host
if ($env:GITHUB_STEP_SUMMARY) { Get-Content (Join-Path $Out 'bench.md') | Add-Content $env:GITHUB_STEP_SUMMARY }
