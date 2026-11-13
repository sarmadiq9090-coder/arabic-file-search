# Smoke test of the exact bahth.exe that would be published: starts the GUI on
# a real Windows machine, lets it index the drive, then drives the command line
# against that index. Fails on a crash report, an early exit or a wrong result.
# -Machine checks the exe is built for that CPU (x64, x86 or arm64), so a
# mislabelled asset can never be published.
param([Parameter(Mandatory)][string]$Exe, [string]$Machine)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
if ($Machine) {
    $bytes = [IO.File]::ReadAllBytes($Exe)
    $pe = [BitConverter]::ToInt32($bytes, 0x3c)
    $code = [BitConverter]::ToUInt16($bytes, $pe + 4)
    $want = @{ x64 = 0x8664; x86 = 0x014c; arm64 = 0xaa64 }[$Machine]
    if ($null -eq $want) { throw "unknown -Machine $Machine" }
    Write-Host ("PE machine {0:x4}, OS arch {1}, process arch {2}" -f $code, `
        [Runtime.InteropServices.RuntimeInformation]::OSArchitecture, `
        [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture)
    if ($code -ne $want) { throw ("{0} is machine {1:x4}, not {2}" -f $Exe, $code, $Machine) }
    # The UI thread's stack: 8 MB since 0.8.2 (a 1 MB stack overflowed on
    # some desktops). SizeOfStackReserve is at optional header + 72.
    $opt = $pe + 24
    $magic = [BitConverter]::ToUInt16($bytes, $opt)
    $stack = if ($magic -eq 0x20b) { [BitConverter]::ToUInt64($bytes, $opt + 72) } else { [BitConverter]::ToUInt32($bytes, $opt + 72) }
    Write-Host ("stack reserve {0:N0} bytes" -f $stack)
    if ($stack -lt 8MB) { throw ("{0} reserves only {1} bytes of stack, not 8 MB" -f $Exe, $stack) }
}
# Task Manager, the taskbar and Explorer show the version resource's
# FileDescription, not the file name: it must say InstaFind (the file stays bahth.exe).
$info = (Get-Item $Exe).VersionInfo
Write-Host ("version resource: description '{0}', product '{1}', version {2}" -f $info.FileDescription, $info.ProductName, $info.FileVersion)
if ($info.FileDescription -ne 'InstaFind' -or $info.ProductName -ne 'InstaFind') { throw "${Exe}: the version resource must name the program InstaFind" }
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
# A runner's drives hold millions of files (toolchains, SDKs); the first scan
# of all of them takes a while.
$started = Get-Date
$deadline = $started.AddMinutes(25)
while ((Get-Date) -lt $deadline -and -not (Test-Path $index)) {
    if ($gui.HasExited) { break }
    Start-Sleep -Seconds 5
}
if (Test-Path $index) { Write-Host ("index saved after {0:N0} s" -f ((Get-Date) - $started).TotalSeconds) }
Start-Sleep -Seconds 15   # let it settle: content indexing, watcher, timers
if ($gui.HasExited) { Fail "the GUI exited by itself (code $($gui.ExitCode))" }
if (Test-Path $crash) { Fail "crash report written:"; Get-Content $crash | Write-Host }
if (Test-Path (Join-Path $data 'log.txt')) { Write-Host '--- log.txt ---'; Get-Content (Join-Path $data 'log.txt') -Tail 40 | Write-Host }

if (Test-Path $index) {
    $r = Run-Cli @('-s', 'تقرير مالي')
    if ($r.Code -ne 0 -or $r.Out -notmatch 'bahth-smoke') { Fail 'Arabic name search did not find the test file' }
    $r = Run-Cli @('-s', 'émoji 😀 report', '--json')
    if ($r.Code -ne 0 -or $r.Out -notmatch 'bahth-smoke') { Fail 'Unicode name search failed' }
    $r = Run-Cli @('-s', 'longxxxxxxxxxx')
    if ($r.Code -ne 0) { Fail 'long name search failed' }
    foreach ($q in @('ext:dll', 'sys: notepad', 'نظام: ext:exe', 'size:>1mb', 'regex:^a.*z$', '"a b"~3', '*', 'ـــ')) {
        $r = Run-Cli @('-s', $q, '--count')
        # Exit 2 is a clean refusal (e.g. tatweel alone is no text); only a
        # crash report or another code counts as a failure here.
        if ($r.Code -gt 2 -or $r.Code -lt 0) { Fail "query '$q' failed with exit $($r.Code)" }
    }
} else {
    Write-Host '::warning::the index was not saved within the time limit; skipped the command-line searches'
}
if (Test-Path $crash) { Fail "crash report written:"; Get-Content $crash | Write-Host }

# The close button hides the window to the tray; the program keeps running.
if (-not $gui.HasExited) {
    $gui.Refresh()
    if ($gui.MainWindowHandle -ne [IntPtr]::Zero) {
        [void]$gui.CloseMainWindow()
        Start-Sleep -Seconds 4
        $gui.Refresh()
        if ($gui.HasExited) { Fail 'the close button quit the program instead of hiding it to the tray' }
        else { Write-Host 'close button: still running in the tray' }
    } else {
        Write-Host '::warning::no main window handle; skipped the close-button check'
    }
}
# Starting with Windows is on by default.
$run = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name BahthSari -ErrorAction SilentlyContinue
if (-not $run -or $run.BahthSari -notmatch '--startup') { Fail 'start with Windows was not turned on' }
if (Test-Path $crash) { Fail "crash report written:"; Get-Content $crash | Write-Host }
if (-not $gui.HasExited) { Stop-Process -Id $gui.Id -Force }
if ($failed) { exit 1 }
Write-Host 'Smoke test passed.'
