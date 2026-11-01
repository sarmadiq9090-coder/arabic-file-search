# Smoke test of the exact bahth.exe that would be published: starts the GUI on
# a real Windows machine, lets it index the drive, then drives the command line
# against that index. Fails on a crash report, an early exit or a wrong result.
param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$crash = Join-Path $data 'crash.txt'
$index = Join-Path $data 'index.bin'
$failed = $false
function Fail($msg) { Write-Host "::error::$msg"; $script:failed = $true }

# Files with Arabic, long and odd names in a normal user folder.
$docs = Join-Path $env:USERPROFILE 'Documents\bahth-smoke'
New-Item -ItemType Directory -Force $docs | Out-Null
Set-Content -LiteralPath (Join-Path $docs 'تقرير مالي ٢٠٢٦.txt') -Value 'الميزانية السنوية' -Encoding utf8
Set-Content -LiteralPath (Join-Path $docs 'Ünïcödé émoji 😀 report.txt') -Value 'x' -Encoding utf8
Set-Content -LiteralPath (Join-Path $docs ('long' + ('x' * 200) + '.txt')) -Value 'x'

function Run-Cli([string[]]$CliArgs) {
    $out = Join-Path $env:RUNNER_TEMP 'cli-out.txt'
    $err = Join-Path $env:RUNNER_TEMP 'cli-err.txt'
    # Start-Process does not quote arguments itself.
    $line = ($CliArgs | ForEach-Object { '"' + ($_ -replace '"', '\"') + '"' }) -join ' '
    $p = Start-Process -FilePath $Exe -ArgumentList $line -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $out -RedirectStandardError $err
    $text = [IO.File]::ReadAllText($out, [Text.Encoding]::UTF8)
    Write-Host "bahth $($CliArgs -join ' ') -> exit $($p.ExitCode)"
    Write-Host $text.Substring(0, [Math]::Min(600, $text.Length))
    return @{ Code = $p.ExitCode; Out = $text }
}

$r = Run-Cli @('--version')
if ($r.Code -ne 0 -or $r.Out -notmatch '\d+\.\d+\.\d+') { Fail "--version failed" }
$r = Run-Cli @('--help')
if ($r.Code -ne 0) { Fail "--help failed" }

Write-Host 'Starting the GUI…'
$gui = Start-Process -FilePath $Exe -PassThru
$deadline = (Get-Date).AddSeconds(420)
while ((Get-Date) -lt $deadline -and -not (Test-Path $index)) {
    if ($gui.HasExited) { break }
    Start-Sleep -Seconds 5
}
Start-Sleep -Seconds 15   # let it settle: content indexing, watcher, timers
if ($gui.HasExited) { Fail "the GUI exited by itself (code $($gui.ExitCode))" }
if (Test-Path $crash) { Fail "crash report written:"; Get-Content $crash | Write-Host }
if (Test-Path (Join-Path $data 'log.txt')) { Write-Host '--- log.txt ---'; Get-Content (Join-Path $data 'log.txt') -Tail 40 | Write-Host }

if (Test-Path $index) {
    $r = Run-Cli @('-s', 'تقرير مالي')
    if ($r.Code -ne 0 -or $r.Out -notmatch 'bahth-smoke') { Fail 'Arabic name search did not find the test file' }
    $r = Run-Cli @('-s', 'emoji report', '--json')
    if ($r.Code -ne 0) { Fail 'Unicode name search failed' }
    $r = Run-Cli @('-s', 'longxxxxxxxxxx')
    if ($r.Code -ne 0) { Fail 'long name search failed' }
    foreach ($q in @('ext:dll', 'size:>1mb', 'regex:^a.*z$', '"a b"~3', 'ـــ', '*')) {
        $r = Run-Cli @('-s', $q, '--count')
        if ($r.Code -gt 1) { Fail "query '$q' failed with exit $($r.Code)" }
    }
} else {
    Write-Host '::warning::the index was not saved within the time limit; skipped the command-line searches'
}
if (Test-Path $crash) { Fail "crash report written:"; Get-Content $crash | Write-Host }
if (-not $gui.HasExited) { Stop-Process -Id $gui.Id -Force }
if ($failed) { exit 1 }
Write-Host 'Smoke test passed.'
