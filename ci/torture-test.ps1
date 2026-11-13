# Torture test of InstaFind on real Windows: hunts for crashes, hangs, a
# corrupt index and runaway memory with hostile input, before a release.
#
#   ./ci/torture-test.ps1 -Exe ./bahth.exe -Group names -Out <folder>
#
# Groups (one job each in .github/workflows/torture-test.yml):
#   names      thousands of hostile file names and paths, then searches
#   tree       hostile folder shapes: 200k files in one folder, loops, odd
#              times and attributes, denied and locked files, a drive that
#              goes away and comes back
#   churn      storms of creates, renames, deletes and moves while the
#              program sits in the tray; then the index must match the disk
#   state      damaged files in %LOCALAPPDATA%\BahthSari, and the process
#              killed 15 times while indexing, saving and reading documents
#   instances  20 quick starts, a second copy, exit paths, sign-out
#   content    document torture for "search inside documents"
#   soak       2 hours (-SoakMinutes) of light churn and searches; memory
#              slope, and the idle trim's log lines
#
# The program is driven through its classic window: since 1.0 users only
# get the new look, so every copy this script starts (and every copy the
# program starts itself, which inherits it) runs with BAHTH_CLASSIC=1, the
# program's own switch for the tests (src/webui/mod.rs, CLASSIC_ENV).
#
# Two kinds of checks, as in ci/ui-test.ps1:
#   core  - must never fail: a crash report, the program gone, a user
#           interface that does not answer, an index that cannot be read
#           after a restart, a file that exists and is not found.
#   probe - a finding, reported as MISS without failing the job.
#
# Rules this script keeps (GitHub's runners are shared and not ours):
#   - only scratch folders (-Root, -Scratch), the program's own data folder
#     and registry keys, a scratch VHD / subst drive and ACLs on scratch
#     folders are touched; everything is removed at the end;
#   - only bahth processes and what the test started are ever killed;
#   - no network: the copy under test lives in a scratch "Scoop" folder, so
#     its updater never contacts GitHub (src/update.rs, managed_by), and
#     every dialog is answered "No";
#   - the system drive keeps at least 10 GB free;
#   - a watchdog ends the run before the job's time limit (-Minutes).
#
# Output in <Out>: report.md (the workflow adds it to the job summary),
# run.log (everything), memory.csv (a sample every 5 s), screenshots,
# copies of log.txt and of any crash.txt.
#
# Windows PowerShell 5.1 on 64-bit Windows with the x64 exe. The C# helpers
# are in ci/torture-test.cs. This file is UTF-8 with a BOM so Windows
# PowerShell reads the Arabic in it.
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][ValidateSet('names', 'tree', 'churn', 'state', 'instances', 'content', 'soak')][string]$Group,
    [string]$Out = (Join-Path $env:RUNNER_TEMP "torture-$Group"),
    # The whole run, set-up and clean-up included, stays under this many
    # minutes (0: 39, under the job's limit of 45; for the soak
    # -SoakMinutes + 30, under its job's limit of 160).
    [int]$Minutes = 0,
    # How long the soak's churn and searches last (the owner's standard
    # for a long run: two hours).
    [int]$SoakMinutes = 120,
    [string]$Root = (Join-Path $env:SystemDrive 'ifx'),
    [string]$Scratch = (Join-Path $env:SystemDrive 'ifx-s')
)
$ErrorActionPreference = 'Stop'
$started = Get-Date
if ($Minutes -le 0) { $Minutes = if ($Group -eq 'soak') { $SoakMinutes + 30 } else { 39 } }
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
foreach ($p in @($Root, $Scratch)) {
    # These two are deleted recursively: never anything but a scratch folder.
    if ($p -notmatch '^[A-Za-z]:\\ifx[A-Za-z0-9-]*$') { throw "refusing to use '$p' as a scratch folder (expected X:\ifx...)" }
}

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'torture-test.cs'))
# (-IgnoreWarnings: with $ErrorActionPreference = 'Stop' a mere compiler
# warning would otherwise end the script.)
try {
    Add-Type -TypeDefinition $source -ReferencedAssemblies 'System.IO.Compression.dll' -IgnoreWarnings
} catch {
    Write-Host "Add-Type with the default references failed ($($_.Exception.Message)); trying explicit ones"
    Add-Type -TypeDefinition $source -ReferencedAssemblies 'System.dll', 'System.Core.dll', 'System.IO.Compression.dll' -IgnoreWarnings
}
[void][TT]::SetProcessDPIAware()

# ---------------------------------------------------------------- constants
$VK = @{ CTRL = 0x11; ALT = 0x12; SHIFT = 0x10; ENTER = 0x0D; ESC = 0x1B; A = 0x41; F = 0x46; DOWN = 0x28; BACK = 0x08; F4 = 0x73 }
$BM_CLICK = 0x00F5; $BM_GETCHECK = 0x00F0
$WM_CLOSE = 0x0010; $WM_CANCELMODE = 0x001F; $WM_QUERYENDSESSION = 0x0011; $WM_ENDSESSION = 0x0016
$WM_MENUCOMMAND = 0x0126; $WM_RBUTTONUP = 0x0205
# native-windows-gui's tray callback message (WM_USER + 102).
$NWG_TRAY = 0x0466
$OptionsPattern = '^Options|^خيارات'
$ExitPattern = '^(Exit|خروج)$'
$IndexingPattern = 'Indexing|جارِ الفهرسة|جاري فهرسة'

$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$crash = Join-Path $data 'crash.txt'
$index = Join-Path $data 'index.bin'
$indexTmp = Join-Path $data 'index.tmp'
$settingsFile = Join-Path $data 'settings.txt'
$logFile = Join-Path $data 'log.txt'
$contentDir = Join-Path $data 'content'
# The copy under test: below "<SCOOP>\apps\", where the program's updater
# leaves updating to Scoop and never goes to the network.
$scoop = Join-Path $Scratch 'scoop'
$App = Join-Path $scoop 'apps\instafind\current\bahth.exe'

$results = New-Object System.Collections.Generic.List[object]
$S = @{}                       # what the steps of a group share
$script:shot = 0
$script:tokN = 1
$script:proc = $null
$script:logMark = 0
$script:ui = $null
$script:crashes = 0
$script:pingMax = 0
$script:pingSlow = 0
$script:pings = 0
$script:hangs = 0
$script:dialogs = @{}
$script:updateLines = @()
$script:locks = New-Object System.Collections.Generic.List[object]
$script:aclPaths = New-Object System.Collections.Generic.List[string]
$script:vhd = $null            # @{ File; Letter }
$script:subst = $null          # drive letter
$script:seedTokens = @()
$softEnd = $started.AddMinutes($Minutes - 3.5)
$runLog = New-Object IO.StreamWriter((Join-Path $Out 'run.log'), $false, (New-Object Text.UTF8Encoding $true))
$runLog.AutoFlush = $true

# ---------------------------------------------------------------- reporting
function Detail([string]$Text) { $runLog.WriteLine(("[{0:HH:mm:ss}] {1}" -f (Get-Date), $Text)) }
function Say([string]$Text) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $Text); Detail $Text }
function Time-Left { return ($softEnd - (Get-Date)).TotalMinutes }
function Out-Of-Time { return ([TM]::Expired -or (Time-Left) -le 0) }
function Short([string]$Text, [int]$Max = 300) {
    $t = ($Text -replace '\s+', ' ').Trim()
    if ($t.Length -gt $Max) { return $t.Substring(0, $Max) + '...' }
    return $t
}
function Record([string]$Area, [string]$Kind, [string]$Check, [bool]$Ok, [string]$Detail = '') {
    $res = if ($Ok) { 'PASS' } elseif ($Kind -eq 'core') { 'FAIL' } else { 'MISS' }
    $results.Add([pscustomobject]@{ Seq = $results.Count; Area = $Area; Kind = $Kind; Check = $Check; Result = $res; Detail = $Detail })
    $line = "[$res] $Area / $Check" + $(if ($Detail) { " - $Detail" } else { '' })
    Detail $line
    # The console stays short: details of what passed are in run.log.
    if ($res -eq 'FAIL') { Write-Host "::error::$(Short $line 600)" }
    elseif ($res -eq 'MISS') { Write-Host "::warning::$(Short $line 400)" }
    else { Write-Host "[PASS] $Area / $Check" }
}
function Note([string]$Area, [string]$Check, [string]$Detail) {
    $results.Add([pscustomobject]@{ Seq = $results.Count; Area = $Area; Kind = 'note'; Check = $Check; Result = 'NOTE'; Detail = $Detail })
    Detail "[NOTE] $Area / $Check - $Detail"
    Write-Host "[NOTE] $Area / $Check - $(Short $Detail 200)"
}
function Shot([string]$Name) {
    if ($script:shot -ge 40) { return }
    $script:shot++
    $file = Join-Path $Out ('{0:D2}-{1}.png' -f $script:shot, ($Name -replace '[^A-Za-z0-9]+', '-').Trim('-'))
    try {
        $b = [Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
        $bmp.Save($file, [Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    } catch { Detail "screenshot $Name failed: $_" }
}
function Wait-Until([scriptblock]$Cond, [double]$Seconds = 10, [int]$EveryMs = 250) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end -and -not [TM]::Expired) {
        $v = & $Cond
        if ($v) { return $v }
        Start-Sleep -Milliseconds $EveryMs
    }
    return $null
}
# One step of a group. A script error in it is reported (with its line) and
# the next step still runs; a step is skipped when the time is nearly up.
function Step([string]$Name, [scriptblock]$Body, [double]$NeedMinutes = 0.5) {
    if ([TM]::Expired -or (Time-Left) -lt $NeedMinutes) {
        Note 'time' "skipped: $Name" ("{0:N1} min left of the {1} min budget; this step wants {2}" -f (Time-Left), $Minutes, $NeedMinutes)
        return
    }
    [TM]::Phase = $Name
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Say "--- $Name"
    try { & $Body }
    catch {
        Record 'harness' 'core' "step '$Name' ran without a script error" $false ("{0} (line {1})" -f $_.Exception.Message, $_.InvocationInfo.ScriptLineNumber)
        Detail $_.ScriptStackTrace
        Shot "error-$Name"
    }
    Detail ("step '{0}' took {1:N0} s; {2:N1} min left" -f $Name, $sw.Elapsed.TotalSeconds, (Time-Left))
}
function Free-GB([string]$Drive = $env:SystemDrive) {
    try { return [Math]::Round((New-Object IO.DriveInfo $Drive).AvailableFreeSpace / 1GB, 1) } catch { return -1 }
}
# True when `NeedGB` more can be written and 10 GB still stay free.
function Have-Space([double]$NeedGB, [string]$What) {
    $free = Free-GB
    if ($free -ge 0 -and $free - $NeedGB -lt 10) {
        Note 'disk' "skipped: $What" "only $free GB free on $env:SystemDrive; the test keeps 10 GB free"
        return $false
    }
    return $true
}
function New-Token { $t = 'zq{0:x6}' -f $script:tokN; $script:tokN++; return $t }

# ---------------------------------------------------------------- windows
function Parse-Win([string]$Line) {
    $p = $Line.Split('|', 8)
    [pscustomobject]@{ Handle = [IntPtr][long]$p[0]; Class = $p[1]; Visible = $p[2] -eq 'True'; X = [int]$p[3]; Y = [int]$p[4]; W = [int]$p[5]; H = [int]$p[6]; Text = $p[7] }
}
function Get-Windows([int]$ProcId) { foreach ($l in [TT]::Windows($ProcId)) { Parse-Win $l } }
function Get-Children($Hwnd) { foreach ($l in [TT]::Children($Hwnd)) { Parse-Win $l } }
function App-Ids { $ids = @(); foreach ($p in [TM]::App()) { $ids += $p.Id; $p.Dispose() }; return $ids }
# The classic main window (it holds the result list, the status bar and the
# Options button), shown or hidden in the tray.
function Main-Window([switch]$Shown) {
    foreach ($id in @(App-Ids)) {
        foreach ($w in @(Get-Windows $id | Where-Object { $_.Class -eq 'NativeWindowsGuiWindow' -and $_.W -gt 300 -and $_.H -gt 200 })) {
            if ($Shown -and -not $w.Visible) { continue }
            $kids = @(Get-Children $w.Handle)
            if (($kids | Where-Object Class -eq 'SysListView32') -and ($kids | Where-Object Class -eq 'msctls_statusbar32') -and ($kids | Where-Object { $_.Class -eq 'Button' -and $_.Text -match $OptionsPattern })) { return $w }
        }
    }
    return $null
}
# The main window's controls, found by class and text (not by visibility:
# the children of a window hidden in the tray count as hidden too).
function Find-Controls($Main) {
    $kids = @(Get-Children $Main.Handle)
    $buttons = @($kids | Where-Object Class -eq 'Button')
    [pscustomobject]@{
        Main    = $Main
        Query   = $kids | Where-Object Class -eq 'Edit' | Sort-Object Y | Select-Object -First 1
        List    = $kids | Where-Object Class -eq 'SysListView32' | Select-Object -First 1
        Status  = $kids | Where-Object Class -eq 'msctls_statusbar32' | Select-Object -First 1
        Content = $buttons | Where-Object { $_.Text -match 'inside documents|داخل المستندات' } | Select-Object -First 1
        Options = $buttons | Where-Object { $_.Text -match $OptionsPattern } | Select-Object -First 1
        Search  = $buttons | Where-Object { $_.Text -match '(Search|بحث)$' -and $_.Text -notmatch 'Stop|إيقاف' } | Select-Object -First 1
        Reindex = $buttons | Where-Object { $_.Text -match 'Update index|تحديث الفهرس' } | Select-Object -First 1
    }
}
function Ui([switch]$Fresh) {
    if (-not $Fresh -and $script:ui -and [TT]::IsWindow($script:ui.Main.Handle)) { return $script:ui }
    $script:ui = $null
    $main = Main-Window
    if ($main) { $script:ui = Find-Controls $main }
    return $script:ui
}
function Status-Text {
    $u = Ui
    if (-not $u -or -not $u.Status) { return '' }
    return (([TT]::StatusText($u.Status.Handle, 0), [TT]::StatusText($u.Status.Handle, 1), [TT]::StatusText($u.Status.Handle, 2)) -join ' | ')
}
function Focus-Window($Hwnd) {
    # Windows only hands the foreground to the process that last had input;
    # a tap of Alt counts as that. It can leave the window in menu mode,
    # where a popup menu closes at once: leave that mode again.
    [TT]::Chord($VK.ALT)
    [void][TT]::SetForegroundWindow($Hwnd)
    Start-Sleep -Milliseconds 300
    [void][TT]::Post($Hwnd, $WM_CANCELMODE, 0, 0)
    Start-Sleep -Milliseconds 200
}

# ---------------------------------------------------------------- the program
function Log-Length { if (Test-Path -LiteralPath $logFile) { return (Get-Item -LiteralPath $logFile).Length } else { return 0 } }
# What the program wrote to log.txt since the last Start-App.
function Log-New {
    try {
        if (-not (Test-Path -LiteralPath $logFile)) { return '' }
        $fs = [IO.File]::Open($logFile, 'Open', 'Read', 'ReadWrite, Delete')
        try {
            # The program empties its log past 64 KB: then all of it is new.
            if ($fs.Length -ge $script:logMark) { [void]$fs.Seek($script:logMark, 'Begin') }
            $sr = New-Object IO.StreamReader($fs, [Text.Encoding]::UTF8)
            return $sr.ReadToEnd()
        } finally { $fs.Dispose() }
    } catch { return '' }
}
function Log-Lines([string]$Pattern) { return @((Log-New) -split "`r?`n" | Where-Object { $_ -match $Pattern }) }
function Keep-Log([string]$Name) {
    if (Test-Path -LiteralPath $logFile) { Copy-Item -LiteralPath $logFile -Destination (Join-Path $Out "log-$Name.txt") -Force -ErrorAction SilentlyContinue }
}
# settings.txt as the program reads it (key=value, no BOM): no first-run
# question and "checked for updates just now". The look: since 1.0 new_ui=0
# is ignored and the classic window comes only with BAHTH_CLASSIC=1 (set
# for every start, see the set-up at the end); the one-time markers
# new_ui_default=1 and new_ui_only=1 are written as the program writes
# them, so that it has nothing to move. The box above the clock is off
# (1.0's default); floating_asked=1 stays for older builds' question.
function Write-Settings([hashtable]$Extra = @{}) {
    New-Item -ItemType Directory -Force $data | Out-Null
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $set = [ordered]@{ floating_asked = 1; move_asked = 1; rating_asked = 1; new_ui = 0; new_ui_failed = 0; new_ui_default = 1; new_ui_only = 1; update_check = $now }
    foreach ($k in $Extra.Keys) { $set[$k] = $Extra[$k] }
    $text = (($set.Keys | ForEach-Object { "$_=$($set[$_])" }) -join "`n") + "`n"
    [IO.File]::WriteAllText($settingsFile, $text, (New-Object Text.ASCIIEncoding))
    Detail "settings.txt written: $($text -replace "`n", ' ')"
}
function Reset-AppData {
    [void](Stop-App)
    if (Test-Path -LiteralPath $data) { [void][TF]::DeleteTree($data) }
    Remove-Item 'HKCU:\Software\BahthSari' -Recurse -Force -ErrorAction SilentlyContinue
}
# Lines about the update check in what the last run logged (there must be
# none: the copy under test is "installed by Scoop").
function Scan-Update { $script:updateLines += @(Log-Lines 'update: (found|up to date|check failed|install)') }
function Start-App([string[]]$AppArgs = @(), [string]$From = $App) {
    Scan-Update
    $script:logMark = Log-Length
    $script:ui = $null
    if ($AppArgs.Count) { $script:proc = Start-Process -FilePath $From -ArgumentList $AppArgs -PassThru }
    else { $script:proc = Start-Process -FilePath $From -PassThru }
    Detail "started $From $($AppArgs -join ' ') as process $($script:proc.Id)"
    return $script:proc
}
# Kills the program as `taskkill /f` does; true when no bahth process is left.
function Stop-App {
    [void][TM]::KillApp()
    $gone = Wait-Until { [TM]::AppCount() -eq 0 } 10 100
    $script:ui = $null
    return [bool]$gone
}
# Answers every message box of the program with No (or Cancel, or OK).
# Never Yes: "Yes" to the crash-report question opens a web page.
function Dismiss-Dialogs([string]$When) {
    $n = 0
    foreach ($id in @(App-Ids)) {
        foreach ($w in @(Get-Windows $id | Where-Object { $_.Visible -and $_.Class -eq '#32770' })) {
            $text = Short ("$($w.Text): " + ((Get-Children $w.Handle | Where-Object { $_.Class -eq 'Static' -and $_.Text } | ForEach-Object Text) -join ' ')) 240
            # Each message once in the report, however often it comes back.
            if (-not $script:dialogs.ContainsKey($text)) { $script:dialogs[$text] = 0; Note 'dialog' "a message box appeared ($When); answered No" $text; Shot 'dialog' }
            $script:dialogs[$text]++
            foreach ($button in 7, 2, 1) {
                $b = [TT]::GetDlgItem($w.Handle, $button)
                if ($b -ne [IntPtr]::Zero) { [void][TT]::Post($b, $BM_CLICK, 0, 0); break }
            }
            $n++
        }
    }
    return $n
}
# A crash report is always a core failure. It is kept in <Out> and removed,
# so the next start does not ask about it.
function Check-Crash([string]$When) {
    if (-not (Test-Path -LiteralPath $crash)) { return $true }
    $script:crashes++
    $text = ''
    try { $text = [IO.File]::ReadAllText($crash, [Text.Encoding]::UTF8) } catch { $text = "(unreadable: $_)" }
    Copy-Item -LiteralPath $crash -Destination (Join-Path $Out ("crash-{0:D2}.txt" -f $script:crashes)) -Force -ErrorAction SilentlyContinue
    Record 'stability' 'core' "no crash report ($When)" $false (Short $text 500)
    Keep-Log ("crash-{0:D2}" -f $script:crashes)
    Shot "crash-$When"
    Remove-Item -LiteralPath $crash -Force -ErrorAction SilentlyContinue
    return $false
}
function Check-Alive([string]$When) {
    $n = [TM]::AppCount()
    if ($n -ge 1) { return $true }
    $code = ''
    try { if ($script:proc -and $script:proc.HasExited) { $code = "exit code $($script:proc.ExitCode)" } } catch { }
    Record 'stability' 'core' "the program is still running ($When)" $false $code
    Keep-Log 'exited'
    return $false
}
# A user interface that does not answer in 15 s is a hang (core). Slower
# than 3 s is counted and reported once, as a finding.
function Check-Responsive([string]$When, [int]$LimitMs = 15000) {
    $u = Ui
    if (-not $u) { return $true }
    $ms = [TT]::Ping($u.Main.Handle, $LimitMs)
    $script:pings++
    if ($ms -lt 0) {
        if ([TM]::AppCount() -eq 0) { return $true }   # gone, not hung: Check-Alive reports it
        $script:hangs++
        # The first few in full; a window that stays hung is not reported a hundred times.
        if ($script:hangs -le 3) {
            Record 'stability' 'core' "the window answers within $([int]($LimitMs / 1000)) s ($When)" $false "no answer to a message for $LimitMs ms"
            Shot "hang-$When"
            Keep-Log "hang-$($script:hangs)"
        } else { Detail "no answer for $LimitMs ms ($When); hang number $($script:hangs)" }
        return $false
    }
    if ($ms -gt $script:pingMax) { $script:pingMax = $ms }
    if ($ms -gt 3000) { $script:pingSlow++; Detail "slow answer: $ms ms ($When)" }
    return $true
}
function Health([string]$When) {
    [void](Check-Crash $When)
    $alive = Check-Alive $When
    if ($alive) { [void](Check-Responsive $When) }
    return $alive
}
# Runs the command-line search of the copy under test (never longer than
# TimeoutS: a command line that hangs is a core failure). Not called "Cli":
# that is a built-in alias of Clear-Item, and an alias wins over a function.
function Run-Cli([string[]]$CliArgs, [int]$TimeoutS = 180) {
    $r = [TT]::Run($App, $CliArgs, $TimeoutS * 1000)
    Detail ("cli {0} -> exit {1} in {2} ms, {3} chars{4}" -f (Short $r.Line 300), $r.Code, $r.Ms, $r.Out.Length, $(if ($r.Err) { "; stderr: $(Short $r.Err 200)" } else { '' }))
    if ($r.TimedOut) { Record 'command line' 'core' "the command line answers within $TimeoutS s" $false (Short $r.Line 200) }
    return $r
}
# token -> the paths whose file name holds it, from ONE search for `Text`
# (every test name holds "zq"). $null when the search itself failed.
function Cli-Map([string]$Scope, [string]$Text = 'zq') {
    $a = @('-s', $Text, '--all', '-n', '5000000')
    if ($Scope) { $a += @('--scope', $Scope) }
    $r = Run-Cli $a
    if ($r.TimedOut -or $r.Code -lt 0 -or $r.Code -gt 1) { $script:cliError = "exit $($r.Code) $(Short $r.Err 200)"; return $null }
    $map = @{}
    foreach ($line in ($r.Out -split "`r?`n")) {
        if (-not $line) { continue }
        $name = $line.Substring($line.LastIndexOf('\') + 1).ToLowerInvariant()
        foreach ($m in [regex]::Matches($name, 'zq[0-9a-f]{6}')) {
            if (-not $map.ContainsKey($m.Value)) { $map[$m.Value] = New-Object System.Collections.Generic.List[string] }
            $map[$m.Value].Add($line)
        }
    }
    return $map
}
# The same for a list of tokens, 40 to a search ("a|b|c" is "any of").
function Cli-Tokens([string[]]$Tokens, [string]$Scope = '') {
    $map = @{}
    for ($i = 0; $i -lt $Tokens.Count; $i += 40) {
        $batch = $Tokens[$i..([Math]::Min($i + 39, $Tokens.Count - 1))]
        $part = Cli-Map $Scope ($batch -join '|')
        if ($null -eq $part) { return $null }
        foreach ($k in $part.Keys) { $map[$k] = $part[$k] }
        if ([TM]::Expired) { return $null }
    }
    return $map
}
# A dozen ordinary files every group makes first. They exist before the
# first index, so every later index must hold them.
function New-Seeds {
    $dir = Join-Path $Root 'seed'
    New-Item -ItemType Directory -Force $dir | Out-Null
    $names = @('تقرير مالي ٢٠٢٦ {T}.txt', 'أحمد - سيرة ذاتية {T}.txt', 'Quarterly Budget 2026 {T}.txt', 'Ünïcödé émoji 😀 {T}.txt', 'محاضرة الفيزياء {T}.pdf',
        'invoice {T}.docx', 'صورة {T}.jpg', 'notes {T}.md', 'data {T}.csv', 'archive {T}.zip', ('long' + ('x' * 180) + ' {T}.txt'), 'مجلد {T}')
    $tokens = @()
    foreach ($n in $names) {
        $t = New-Token
        $path = Join-Path $dir $n.Replace('{T}', $t)
        if ($n -eq 'مجلد {T}') { New-Item -ItemType Directory -Force -Path $path | Out-Null }
        else { [IO.File]::WriteAllText($path, "seed $t الميزانية السنوية", (New-Object Text.UTF8Encoding $false)) }
        $tokens += $t
    }
    $script:seedTokens = $tokens
    Detail "seed files: $($tokens -join ' ')"
}
# Core: the saved index can be read and holds every seed file.
function Check-Seeds([string]$When, [string]$Kind = 'core') {
    if (-not (Test-Path -LiteralPath $index)) { Record 'index' $Kind "the saved index holds the seed files ($When)" $false 'there is no index.bin'; return $false }
    $map = Cli-Tokens $script:seedTokens (Join-Path $Root 'seed')
    if ($null -eq $map) { Record 'index' $Kind "the saved index can be read ($When)" $false $script:cliError; return $false }
    $missing = @($script:seedTokens | Where-Object { -not $map.ContainsKey($_) })
    Record 'index' $Kind "the saved index holds the seed files ($When)" ($missing.Count -eq 0) $(if ($missing.Count) { "missing: $($missing -join ' ')" } else { "$($script:seedTokens.Count) found" })
    return ($missing.Count -eq 0)
}
# Waits for the main window after a start (hidden counts for --startup).
function Wait-Main([string]$When, [int]$Seconds = 30, [switch]$Quiet) {
    $t = Get-Date
    $main = Wait-Until { Main-Window } $Seconds
    if (-not $Quiet -or -not $main) {
        Record 'start' 'core' "the main window is created ($When)" ($null -ne $main) ("{0:N1} s" -f ((Get-Date) - $t).TotalSeconds)
    }
    if (-not $main) {
        Shot "no-window-$When"; Keep-Log 'no-window'
        # The new look's window instead (BAHTH_CLASSIC not taken)? Then the
        # failure is the test switch's, not the classic window's.
        $other = @(foreach ($id in @(App-Ids)) { Get-Windows $id | Where-Object { $_.Visible -and $_.Class -eq 'NativeWindowsGuiWindow' -and $_.W -gt 300 -and $_.H -gt 200 } })
        Detail ("no classic window ({0}): BAHTH_CLASSIC={1}; other main windows on screen: {2}; log: {3}" -f $When, $env:BAHTH_CLASSIC, (($other | ForEach-Object { "'$($_.Text)' $($_.W)x$($_.H)" }) -join ', '), ((Log-Lines 'webui') -join ' | '))
    }
    return $main
}
# Waits until the first index is saved. A count that stops growing for five
# minutes is a hang (core); a slow runner that is still counting when the
# time is up is only noted.
function Wait-Index([double]$MaxMinutes = 22) {
    $t0 = Get-Date
    $limit = [Math]::Min($MaxMinutes, (Time-Left) - 4)
    $last = -1; $changes = 0; $lastChange = Get-Date; $next = (Get-Date).AddSeconds(60); $hung = $false; $part = ''
    while (-not (Test-Path -LiteralPath $index)) {
        if ([TM]::Expired -or ((Get-Date) - $t0).TotalMinutes -ge $limit -or [TM]::AppCount() -eq 0) { break }
        Start-Sleep -Seconds 3
        [void](Dismiss-Dialogs 'indexing')
        $u = Ui
        $part = if ($u -and $u.Status) { [string][TT]::StatusText($u.Status.Handle, 2) } else { '' }
        $digits = $part -replace '[^\d]', ''
        $n = if ($digits) { [long]$digits } else { -1 }
        if ($n -ne $last) { $last = $n; $changes++; $lastChange = Get-Date }
        elseif ($changes -ge 3 -and ((Get-Date) - $lastChange).TotalSeconds -gt 300) { $hung = $true; break }
        if ((Get-Date) -ge $next) {
            $next = (Get-Date).AddSeconds(60)
            Say ("  indexing for {0:N0} s: {1}" -f ((Get-Date) - $t0).TotalSeconds, $part)
        }
    }
    $secs = ((Get-Date) - $t0).TotalSeconds
    $line = (Log-Lines 'index build:') -join ' | '
    if (Test-Path -LiteralPath $index) {
        Record 'index' 'core' 'the drives are indexed and the index is saved' $true ("{0:N0} s; {1}" -f $secs, $line)
        Start-Sleep -Seconds 3
        return $true
    }
    Keep-Log 'no-index'
    Shot 'no-index'
    if ([TM]::AppCount() -eq 0) { Record 'index' 'core' 'the program survives the first indexing' $false ("gone after {0:N0} s" -f $secs) }
    elseif ($hung) { Record 'index' 'core' 'indexing does not hang' $false ("the count stayed at '{0}' for 5 minutes" -f $part) }
    else { Record 'index' 'probe' 'the first index is saved in time' $false ("not after {0:N0} s (limit {1:N0} min); status '{2}'. A slow runner disk does this; the searches that need the index are skipped" -f $secs, $limit, $part) }
    return $false
}
# A clean first start as every group begins: fresh data folder, seeded
# settings, then the first index.
function Fresh-Start([string[]]$AppArgs = @(), [double]$IndexMinutes = 22, [hashtable]$Settings = @{}) {
    Reset-AppData
    Write-Settings $Settings
    [void](Start-App $AppArgs)
    $main = Wait-Main 'first start'
    Start-Sleep -Seconds 2
    $asked = @(Log-Lines 'webui: the classic window was asked for')
    Record 'setup' 'probe' 'the program took the tests'' switch for the classic window (BAHTH_CLASSIC=1)' ($asked.Count -gt 0) ((Log-Lines 'webui') -join ' | ')
    [void](Dismiss-Dialogs 'first start')
    Shot 'first-start'
    [void](Health 'first start')
    $S.indexed = Wait-Index $IndexMinutes
    [void](Health 'after the first index')
    if ($S.indexed) { [void](Check-Seeds 'after the first index') }
    return $S.indexed
}
# Waits until a (re)started program has loaded its saved index.
function Wait-Loaded([int]$Seconds = 60) {
    $line = Wait-Until { Log-Lines 'index load: \d+ entries' | Select-Object -Last 1 } $Seconds
    return $line
}

# ---------------------------------------------------------------- the window as a user
# Rows of the result list: column 0 (the name) of the first rows.
function List-Names([int]$Max = 40) {
    $u = Ui
    if (-not $u -or -not $u.List) { return @() }
    $n = [Math]::Min([TT]::ListCount($u.List.Handle), $Max)
    $names = @()
    for ($i = 0; $i -lt $n; $i++) { $t = [TT]::ListText($u.List.Handle, $i, 0); if ($null -ne $t) { $names += $t } }
    return $names
}
# Puts `Query` in the search box (as typing does: the list follows) and
# waits for a row that holds `Expect`. Works while the window is hidden.
function Gui-Find([string]$Query, [string]$Expect, [double]$Wait = 15) {
    $u = Ui
    $r = [pscustomobject]@{ Sent = $false; Hit = $null; Rows = -1; Seconds = 0.0; Status = '' }
    if (-not $u -or -not $u.Query) { return $r }
    $t = Get-Date
    $r.Sent = [TT]::SetText($u.Query.Handle, $Query, 10000)
    if ($r.Sent -and $Expect) {
        $r.Hit = Wait-Until { List-Names 30 | Where-Object { $_ -and $_.ToLowerInvariant().Contains($Expect) } | Select-Object -First 1 } $Wait 200
    }
    $r.Seconds = ((Get-Date) - $t).TotalSeconds
    $r.Rows = if ($u.List) { [TT]::ListCount($u.List.Handle) } else { -1 }
    if (-not $r.Hit) { $r.Status = Short (Status-Text) 160 }
    return $r
}
function Gui-Clear { $u = Ui; if ($u -and $u.Query) { [void][TT]::SetText($u.Query.Handle, '', 5000) } }
# Core: the window (its own index in memory) finds a seed file.
function Gui-Seed([string]$When, [string]$Kind = 'core') {
    $tok = $script:seedTokens[0]
    $r = Gui-Find $tok $tok 25
    Record 'window' $Kind "the window finds a seed file ($When)" ($null -ne $r.Hit) ("{0:N1} s, {1} rows{2}" -f $r.Seconds, $r.Rows, $(if ($r.Status) { "; status: $($r.Status)" } else { '' }))
    Gui-Clear
    return ($null -ne $r.Hit)
}
# Brings the window back as a user does: by starting the program again.
function Show-Main {
    $w = Main-Window -Shown
    if ($w) { return $w }
    for ($try = 1; $try -le 2; $try++) {
        $again = Start-Process -FilePath $App -PassThru
        [void]$again.WaitForExit(20000)
        $w = Wait-Until { Main-Window -Shown } 15
        if ($w) { return $w }
        Detail "Show-Main: no window on screen after start $try; processes: $((App-Ids) -join ', ')"
    }
    return $null
}
# Opens a popup menu with `Open`, finds the item matching `Pattern`, closes
# the menu and reports the choice the way the menu itself does
# (WM_MENUCOMMAND): no keyboard focus is needed.
function Menu-Command($Main, [scriptblock]$Open, [string]$Pattern, [string]$What) {
    for ($try = 1; $try -le 3; $try++) {
        & $Open
        $mw = Wait-Until { $m = @([TT]::MenuWindows()); if ($m.Count) { $m[0] } } 4 150
        if (-not $mw) { Detail "$What did not open (try $try)"; continue }
        $h = [IntPtr][long]$mw
        $found = [TT]::MenuFind($h, $Pattern)
        $items = ([TT]::MenuItems($h)) -join ' / '
        Detail "${What}: $items"
        [void][TT]::Send($Main.Handle, $WM_CANCELMODE, 0, 0, 3000)
        [void](Wait-Until { @([TT]::MenuWindows()).Count -eq 0 } 3 100)
        if ($found) { [void][TT]::Post($Main.Handle, $WM_MENUCOMMAND, $found[1], $found[0]); return $true }
        Detail "$What has no item matching $Pattern"
        return $false
    }
    return $false
}
# "Exit" from the tray menu (or, failing that, from the Options menu).
# Seconds until every bahth process is gone; -1 no window, -2 no menu,
# -3 still running after 30 s.
function Exit-App([string]$When) {
    $main = Main-Window
    if (-not $main) { return -1 }
    $t = Get-Date
    $how = 'tray menu'
    $ok = Menu-Command $main { [void][TT]::Post($main.Handle, $NWG_TRAY, 0, $WM_RBUTTONUP) } $ExitPattern 'tray menu'
    if (-not $ok) {
        $u = Ui -Fresh
        if ($u -and $u.Options) {
            $how = 'Options menu'
            $t = Get-Date
            $ok = Menu-Command $main { [void][TT]::Post($u.Options.Handle, $BM_CLICK, 0, 0) } $ExitPattern 'Options menu'
        }
    }
    if (-not $ok) { Detail "exit ($When): no menu offered Exit"; return -2 }
    $gone = Wait-Until { [TM]::AppCount() -eq 0 } 30 100
    $secs = ((Get-Date) - $t).TotalSeconds
    Detail ("exit ($When) through the {0}: {1}" -f $how, $(if ($gone) { "gone after {0:N1} s" -f $secs } else { 'still running after 30 s' }))
    $script:ui = $null
    if (-not $gone) { return -3 }
    return $secs
}
# Records an Exit-App result: core "exits promptly", probe when the menu
# could not be driven (a limit of the test, not of the program).
function Record-Exit([string]$When, [double]$Secs) {
    if ($Secs -eq -2 -or $Secs -eq -1) { Record 'exit' 'probe' "Exit could be chosen from a menu ($When)" $false $(if ($Secs -eq -1) { 'no main window' } else { 'neither the tray menu nor the Options menu opened' }); return $false }
    Record 'exit' 'core' "Exit ends the program within 15 s ($When)" ($Secs -ge 0 -and $Secs -lt 15) $(if ($Secs -eq -3) { 'still running after 30 s' } else { "{0:N1} s" -f $Secs })
    return ($Secs -ge 0)
}
# Runs one long file job in the background and watches the program
# meanwhile. Returns the job's result.
function Run-Job([string]$What, [string]$Op, [string]$A, [string]$B, [int[]]$N, [switch]$NoApp) {
    [TJ]::Start($Op, $A, $B, $N)
    while ([TJ]::Busy) {
        Start-Sleep -Milliseconds 1500
        if (-not $NoApp) { [void](Check-Responsive $What) }
    }
    $res = [TJ]::Result
    $secs = [Math]::Max([TJ]::Seconds, 0.001)
    Note 'files' $What ("{0:N0} in {1:N1} s ({2:N0} a minute){3}" -f $res, $secs, ($res / $secs * 60), $(if ([TJ]::Error) { "; error: $([TJ]::Error)" } else { '' }))
    if (-not $NoApp) { [void](Health $What) }
    return $res
}

# ---------------------------------------------------------------- drives
function Free-Letter {
    $used = @([IO.DriveInfo]::GetDrives() | ForEach-Object { $_.Name.Substring(0, 1).ToUpperInvariant() })
    foreach ($l in 'T', 'U', 'V', 'W', 'X', 'Y', 'R', 'Q', 'P') { if ($used -notcontains $l) { return $l } }
    return $null
}
function Diskpart([string[]]$Lines, [string]$What) {
    $f = Join-Path $Scratch 'diskpart.txt'
    [IO.File]::WriteAllLines($f, $Lines, (New-Object Text.ASCIIEncoding))
    $r = [TT]::Run('diskpart.exe', @('/s', $f), 150000)
    Detail "diskpart ($What): exit $($r.Code): $(Short $r.Out 600) $(Short $r.Err 200)"
    return $r
}
# A small scratch VHD as its own drive letter. $null when the runner cannot.
function New-Vhd([int]$SizeMB = 300) {
    $letter = Free-Letter
    if (-not $letter) { return $null }
    $file = Join-Path $Scratch 'scratch.vhd'
    $r = Diskpart @("create vdisk file=`"$file`" maximum=$SizeMB type=expandable", "select vdisk file=`"$file`"", 'attach vdisk', 'online disk noerr', 'attributes disk clear readonly noerr', 'convert mbr noerr', 'create partition primary', 'format fs=ntfs quick label=IFXVHD', "assign letter=$letter") 'create the VHD'
    $ok = Wait-Until { Test-Path "${letter}:\" } 20
    if (-not $ok) { Note 'drives' 'scratch VHD' "could not be created (diskpart exit $($r.Code)): $(Short $r.Out 200)"; return $null }
    $script:vhd = @{ File = $file; Letter = $letter }
    Note 'drives' 'scratch VHD' "${letter}: ($SizeMB MB, NTFS)"
    return $letter
}
function Detach-Vhd {
    if (-not $script:vhd) { return $false }
    $r = Diskpart @("select vdisk file=`"$($script:vhd.File)`"", 'detach vdisk') 'detach the VHD'
    return [bool](Wait-Until { -not (Test-Path "$($script:vhd.Letter):\") } 20)
}
function Attach-Vhd {
    if (-not $script:vhd) { return $false }
    $l = $script:vhd.Letter
    [void](Diskpart @("select vdisk file=`"$($script:vhd.File)`"", 'attach vdisk') 'attach the VHD')
    if (-not (Wait-Until { Test-Path "${l}:\" } 15)) {
        # Windows did not give the letter back by itself.
        [void](Diskpart @("select vdisk file=`"$($script:vhd.File)`"", 'select partition 1', "assign letter=$l") 'assign the letter again')
    }
    return [bool](Wait-Until { Test-Path "${l}:\" } 15)
}
function New-Subst([string]$Target) {
    $letter = Free-Letter
    if (-not $letter) { return $null }
    $r = [TT]::Run('subst.exe', @("${letter}:", $Target), 20000)
    if ($r.Code -ne 0 -or -not (Test-Path "${letter}:\")) { Note 'drives' 'subst drive' "could not be made: exit $($r.Code) $(Short $r.Out 100)"; return $null }
    $script:subst = $letter
    return $letter
}
function Remove-Subst { if ($script:subst) { [void][TT]::Run('subst.exe', @("$($script:subst):", '/d'), 20000) } }
# A second fixed drive the program indexes: a real one when the runner has
# it (D:), else a scratch VHD. Returns "X:\" or $null.
function Second-Drive {
    foreach ($d in [IO.DriveInfo]::GetDrives()) {
        if ($d.DriveType -eq 'Fixed' -and $d.IsReady -and $d.Name -ne "$env:SystemDrive\" -and $d.AvailableFreeSpace -gt 3GB) {
            try {
                $probe = Join-Path $d.Name 'ifx-probe.tmp'
                [IO.File]::WriteAllText($probe, 'x'); Remove-Item -LiteralPath $probe -Force
                return $d.Name
            } catch { Detail "drive $($d.Name) is not writable: $_" }
        }
    }
    $l = New-Vhd 300
    if ($l) { return "${l}:\" }
    return $null
}
function Release-Locks {
    foreach ($l in $script:locks) { try { $l.Dispose() } catch { } }
    $script:locks.Clear()
}
function Deny-Read([string]$Path, [switch]$Folder) {
    $rule = if ($Folder) { '*S-1-1-0:(OI)(CI)(RX)' } else { '*S-1-1-0:(R)' }
    $r = [TT]::Run('icacls.exe', @($Path, '/deny', $rule), 30000)
    if ($r.Code -eq 0) { $script:aclPaths.Add($Path); return $true }
    Detail "icacls deny on $Path failed: $(Short $r.Out 200) $(Short $r.Err 200)"
    return $false
}
function Undo-Acls {
    foreach ($p in $script:aclPaths) {
        $r = [TT]::Run('icacls.exe', @($p, '/remove:d', '*S-1-1-0'), 30000)
        if ($r.Code -ne 0) { Detail "icacls remove on $p failed: $(Short $r.Out 200)" }
    }
    $script:aclPaths.Clear()
}

# ================================================================ names
function Group-Names {
    $S.dir = Join-Path $Root 'names'
    Step 'generate hostile names' {
        [void][TF]::MkDirs($S.dir)
        $S.items = New-Object System.Collections.Generic.List[object]
        $failed = New-Object System.Collections.Generic.List[string]
        $bytes = [Text.Encoding]::ASCII.GetBytes("name torture`r`n")
        $cat = [TN]::Catalogue()
        # A case-sensitive folder for names that differ only by case (needs
        # an NTFS feature that is not on every Windows).
        $cs = Join-Path $S.dir 'case-sensitive'
        [void][TF]::MkDirs($cs)
        $r = [TT]::Run('fsutil.exe', @('file', 'setCaseSensitiveInfo', $cs, 'enable'), 30000)
        $S.cs = ($r.Code -eq 0)
        Note 'names' 'case-sensitive folder' $(if ($S.cs) { 'enabled' } else { "not available here (fsutil: $(Short $r.Out 120)); names that differ only by case collapse to one file" })
        $suffixes = @('', ' مجلد عربي', ' 😀 emoji', ' (copy) [1] {x}', ' with  two  spaces', " tatweel ـــ", ' ünïcödé')
        $made = 0
        for ($round = 1; $round -le 12; $round++) {
            $rd = Join-Path $S.dir ("r{0:d2}{1}" -f $round, $suffixes[$round % $suffixes.Count])
            [void][TF]::MkDirs($rd)
            foreach ($e in $cat) {
                $tok = New-Token
                $item = [pscustomobject]@{ Token = $tok; Kind = $e[0]; Flags = $e[1]; Round = $round; Made = 0; Kids = 0; Dir = $rd; Path = '' }
                if ($e[1] -eq 'dev') {
                    $dev = Join-Path $rd "dev $tok"
                    if ([TF]::MkDir($dev)) {
                        $item.Made = 1; $item.Dir = $dev
                        for ($i = 2; $i -lt $e.Length; $i++) {
                            if ([TF]::Write("$dev\$($e[$i])", $bytes, $false)) { $item.Kids++; $made++ } else { $failed.Add("device name $($e[$i]): error $([TF]::LastError)") }
                        }
                    }
                } else {
                    $target = $rd
                    if ($e[1] -eq 'cs' -and $S.cs) { $target = Join-Path $cs ("r{0:d2}" -f $round); [void][TF]::MkDirs($target) }
                    for ($i = 2; $i -lt $e.Length; $i++) {
                        if ([TF]::Write(("$target\" + $e[$i].Replace('{T}', $tok)), $bytes, $false)) { $item.Made++; $made++ }
                        elseif ([TF]::LastError -ne 80 -and [TF]::LastError -ne 183) { $failed.Add("$($e[0]): error $([TF]::LastError)") }
                    }
                }
                $S.items.Add($item)
            }
        }
        # Random mixes of scripts, marks, emoji and symbols.
        $rnd = Join-Path $S.dir 'random'
        for ($i = 0; $i -lt 2500; $i++) {
            $tok = New-Token
            $sub = Join-Path $rnd ('{0:d2}' -f ($i % 20))
            if ($i -lt 20) { [void][TF]::MkDirs($sub) }
            $path = "$sub\" + [TN]::RandomName(1000 + $i, $tok)
            $item = [pscustomobject]@{ Token = $tok; Kind = 'random'; Flags = ''; Round = 0; Made = 0; Kids = 0; Dir = $sub; Path = $path }
            if ([TF]::Write($path, $bytes, $false)) { $item.Made = 1; $made++ } else { $failed.Add("random name: error $([TF]::LastError)") }
            $S.items.Add($item)
        }
        # Paths longer than 260, up to the 32,767 limit.
        $S.deep = @([TF]::DeepChain((Join-Path $S.dir 'deep'), 200, 32740, [int[]]@(270, 520, 1100, 4200, 16500, 30000), 0x0F0000))
        foreach ($line in $S.deep) {
            $f = $line.Split('|', 3)
            $S.items.Add([pscustomobject]@{ Token = $f[0]; Kind = "path of $($f[1]) characters"; Flags = ''; Round = 0; Made = 1; Kids = 0; Dir = ''; Path = $f[2] })
            $made++
        }
        # Folders only \\?\ can make, each with a file inside. Windows drops
        # a trailing dot or space from an ordinary path, so what is inside
        # such a folder may be out of the program's reach: a probe.
        $special = Join-Path $S.dir 'special'
        [void][TF]::MkDirs($special)
        $specs = @(@('folder with a trailing dot', '{T} folder dot.', ''), @('folder with a trailing space', '{T} folder space ', ''),
            @('folder named with a lone surrogate', ("{T} lone" + [char]0xD800), 'lone'), @('folder of 255 characters', ('{T} ' + ('d' * 243)), ''))
        foreach ($sp in $specs) {
            $dt = New-Token; $ft = New-Token
            $d = "$special\" + $sp[1].Replace('{T}', $dt)
            if ([TF]::MkDir($d)) {
                $S.items.Add([pscustomobject]@{ Token = $dt; Kind = "$($sp[0])"; Flags = $sp[2]; Round = 0; Made = 1; Kids = 0; Dir = $d; Path = $d })
                if ([TF]::Write("$d\inside $ft.txt", $bytes, $false)) {
                    $S.items.Add([pscustomobject]@{ Token = $ft; Kind = "a file inside a $($sp[0])"; Flags = 'probe'; Round = 0; Made = 1; Kids = 0; Dir = $d; Path = "$d\inside $ft.txt" })
                }
            } else { $failed.Add("$($sp[0]): error $([TF]::LastError)") }
        }
        # Alternate data streams: on a file and on a folder.
        $ads = Join-Path $S.dir 'ads'
        [void][TF]::MkDirs($ads)
        $tok = New-Token
        $p = "$ads\$tok with streams.txt"
        if ([TF]::Write($p, $bytes, $false)) {
            $n = 0
            foreach ($stream in ':secret', ':Zone.Identifier', (':' + ('s' * 200)), ':تيار', ':😀:$DATA') { if ([TF]::Write("$p$stream", $bytes, $true)) { $n++ } }
            [void][TF]::Write("${ads}:folder-stream", $bytes, $true)
            $S.items.Add([pscustomobject]@{ Token = $tok; Kind = "file with $n alternate data streams"; Flags = ''; Round = 0; Made = 1; Kids = 0; Dir = $ads; Path = $p })
        }
        $fails = @($failed | Group-Object | Sort-Object Count -Descending | Select-Object -First 6 | ForEach-Object { "$($_.Count)x $($_.Name)" })
        Note 'names' 'generated' ("{0:N0} files and folders with hostile names for {1:N0} tokens; could not be created: {2} ({3})" -f $made, $S.items.Count, $failed.Count, ($fails -join '; '))
        [IO.File]::WriteAllLines((Join-Path $Out 'names-manifest.txt'), [string[]]@($S.items | ForEach-Object { "$($_.Token)`t$($_.Made)`t$($_.Flags)`t$($_.Kind)" }), (New-Object Text.UTF8Encoding $true))
    }
    Step 'start and index' { [void](Fresh-Start) } 8

    Step 'command line finds every name' {
        if (-not $S.indexed) { Note 'names' 'skipped' 'no index'; return }
        $map = Cli-Map $S.dir
        if ($null -eq $map) { Record 'names' 'core' 'the command line lists the hostile names' $false $script:cliError; return }
        $bad = @(); $lone = @(); $probes = @(); $checked = 0
        foreach ($it in $S.items) {
            if ($it.Made -eq 0) { continue }
            $n = if ($map.ContainsKey($it.Token)) { $map[$it.Token].Count } else { 0 }
            if ($it.Flags -eq 'lone') { $lone += "$($it.Kind): $n of $($it.Made)" }
            elseif ($it.Flags -eq 'probe') { if ($n -ne $it.Made) { $probes += "$($it.Kind) [$($it.Token)]" } }
            else { $checked++; if ($n -ne $it.Made) { $bad += "$($it.Kind) [$($it.Token)] on disk $($it.Made), found $n" } }
        }
        Record 'names' 'core' 'every hostile name on disk is found by the command line (and none twice)' ($bad.Count -eq 0) ("{0:N0} tokens checked; wrong {1}: {2}" -f $checked, $bad.Count, (($bad | Select-Object -First 8) -join '; '))
        if ($bad.Count) { [IO.File]::WriteAllLines((Join-Path $Out 'names-wrong.txt'), [string[]]$bad, (New-Object Text.UTF8Encoding $true)) }
        $loneFound = @($lone | Where-Object { $_ -notmatch ': 0 of' })
        Record 'names' 'probe' 'names with a lone UTF-16 surrogate are found' ($lone.Count -gt 0 -and $loneFound.Count -eq $lone.Count) ("found {0} of {1} (the index keeps only names that are valid Unicode)" -f $loneFound.Count, $lone.Count)
        Record 'names' 'probe' 'files inside folders that end with a dot or a space are found' ($probes.Count -eq 0) ($probes -join '; ')
        # Reserved device names (CON, NUL, COM1...), which hold no token.
        $dev = $S.items | Where-Object { $_.Flags -eq 'dev' -and $_.Made -eq 1 } | Select-Object -First 1
        if ($dev) {
            $r = Run-Cli @('-s', 'con|prn|aux|nul|com|lpt|clock', '--scope', $dev.Dir, '--all', '--count')
            $n = 0; [void][int]::TryParse($r.Out.Trim(), [ref]$n)
            Record 'names' 'probe' 'files named like devices (CON, NUL, COM1...) are indexed' ($n -eq $dev.Kids) "$n of $($dev.Kids) listed"
        }
        # Output formats over the hostile names.
        $r1 = $S.items | Where-Object { $_.Round -eq 1 -and $_.Flags -eq '' } | Select-Object -First 1
        if ($r1) {
            $r = Run-Cli @('-s', 'zq', '--scope', $r1.Dir, '--all', '--json')
            # (Windows PowerShell hands back a JSON array as one object: count its items.)
            $objects = -1
            try { $objects = @(ConvertFrom-Json $r.Out | ForEach-Object { $_ }).Count } catch { Detail "JSON did not parse: $_" }
            Record 'names' 'probe' 'the JSON output over hostile names parses' ($r.Code -eq 0 -and $objects -gt 50) "exit $($r.Code), $objects objects, $($r.Out.Length) characters"
            $r = Run-Cli @('-s', 'zq', '--scope', $r1.Dir, '--all', '--csv', '--sort', 'name', '--desc')
            Record 'names' 'probe' 'the CSV output over hostile names has its header and a row for each' ($r.Code -eq 0 -and $r.Out -match 'path,size,modified' -and ($r.Out -split "`n").Count -gt 50) "exit $($r.Code), $(($r.Out -split "`n").Count) lines"
        }
        [void](Check-Crash 'command-line searches for hostile names')
    }
    Step 'hostile queries on the command line' {
        if (-not $S.indexed) { return }
        $rlo = [string][char]0x202E
        $queries = @('!', '-', '|', '||', '|a|', '"', '""', '"unterminated', '"a b"~3', '"a b"~99999999999', 'ext:', 'ext:,', 'ext:;;;', '*.', '.', '..', '...',
            'size:', 'size:>', 'size:>1', 'size:<0', 'size:99999999999999999999gb', 'size:1..', 'size:-1', 'dm:', 'dm:0000-00-00', 'dm:9999999d', 'dm:2024-13-45', 'dm:today..', 'dm:1601',
            'regex:', 'regex:(', 'regex:(a+)+$', 'regex:\p{Arabic}+', 'regex:.{1000}', 're:.*', 're:[', 'ww:', 'ww:a', 'fuzzy:', ('fuzzy:' + ('a' * 300)), 'root:كتب', 'root:', 'franco:3a2a7a5', 'franco:',
            'folder:', 'مجلد:', 'sys:', 'نظام:', '*', '*.*', '?', '\', '/', 'C:\', '\\?\C:\', ('a' * 5000), ('ab ' * 1500), ('|' * 255), ('"' * 99), ($rlo + 'txt.exe'), ([string][char]0x200B + [char]0x200D),
            'ـــ', 'ً', '😀', ('😀' * 200), '%s%s%s%n', '$(calc)', '`whoami`', '&&', '--', 'تقرير|report|!x|-y', 'ext:txt size:>0 dm:1d ww:a regex:a', ('zq000001|' * 400))
        $bad = @(); $slow = @()
        foreach ($q in $queries) {
            if (Out-Of-Time) { break }
            $r = Run-Cli @('-s', $q, '--count') 90
            if ($r.Code -lt 0 -or $r.Code -gt 2) { $bad += "'$(Short $q 40)' -> exit $($r.Code)" }
            if ($r.Ms -gt 20000) { $slow += "'$(Short $q 40)' $($r.Ms) ms" }
        }
        Record 'queries' 'core' 'hostile queries on the command line end with exit 0, 1 or 2' ($bad.Count -eq 0) ("{0} queries; wrong: {1}" -f $queries.Count, ($bad -join '; '))
        Record 'queries' 'probe' 'no query takes longer than 20 s on the command line' ($slow.Count -eq 0) ($slow -join '; ')
        foreach ($a in @(@('-n', '18446744073709551615', '-s', 'zq'), @('--max=0', '-s', 'zq'), @('--sort', 'date', '--desc', '--csv', '-s', 'zq', '-n', '5'), @('--scope', 'Q:\nowhere\', '-s', 'zq'), @('--scope', ('C:\' + ('x' * 30000)), '-s', 'zq'), @('--json', '--count', '-s', 'x'), @('-s'), @('-s', 'zq', '--bogus'), @('-s', 'zq', '-n', 'many'), @('-s', 'zq', '--sort', 'colour'))) {
            $r = Run-Cli $a 90
            if ($r.Code -lt 0 -or $r.Code -gt 2) { Record 'queries' 'core' "odd options end cleanly ($(Short ($a -join ' ') 60))" $false "exit $($r.Code)" }
        }
        [void](Check-Crash 'hostile queries on the command line')
        # An argument that is not valid Unicode: no shell passes one, so a
        # crash report here is a finding, not a failure.
        $r = Run-Cli @('-s', ('zq' + [char]0xD800 + 'x'), '--count') 60
        $crashed = Test-Path -LiteralPath $crash
        if ($crashed) { Move-Item -LiteralPath $crash -Destination (Join-Path $Out 'crash-cli-lone-surrogate.txt') -Force }
        Record 'queries' 'probe' 'a command-line argument with a lone surrogate ends cleanly (no crash report)' (-not $crashed -and $r.Code -ge 0 -and $r.Code -le 2) "exit $($r.Code); crash report written: $crashed"
    }
    Step 'the window finds hostile names' {
        if (-not $S.indexed) { return }
        [void](Show-Main)
        $sample = @($S.items | Where-Object { $_.Round -eq 1 -and $_.Made -gt 0 -and $_.Flags -eq '' })
        $sample += @($S.items | Where-Object { $_.Kind -like 'path of*' })
        $sample += @($S.items | Where-Object { $_.Kind -eq 'random' -and $_.Made -eq 1 } | Select-Object -First 12)
        $miss = @(); $n = 0; $slowest = 0.0
        foreach ($it in $sample) {
            if (Out-Of-Time) { break }
            $r = Gui-Find $it.Token $it.Token 15
            $n++
            if ($r.Seconds -gt $slowest) { $slowest = $r.Seconds }
            if (-not $r.Hit) { $miss += "$($it.Kind) [$($it.Token)] ($($r.Status))"; if ($miss.Count -le 3) { Shot "gui-miss-$($it.Token)" } }
            else {
                # The other columns too: the path (up to 32,000 characters), size and date.
                $u = Ui
                foreach ($col in 1, 2, 3) { if ($null -eq [TT]::ListText($u.List.Handle, 0, $col)) { $miss += "$($it.Kind): column $col did not answer" } }
            }
            if (-not (Check-Alive "window search for $($it.Kind)")) { break }
        }
        Shot 'names-window'
        Record 'names' 'core' 'the window finds hostile names (typed in its search box)' ($n -gt 0 -and $miss.Count -eq 0) ("{0} names, slowest {1:N1} s; not found {2}: {3}" -f $n, $slowest, $miss.Count, (($miss | Select-Object -First 6) -join '; '))
        [void](Health 'window searches for hostile names')
        # Text no keyboard types, straight into the search box.
        $rlo = [string][char]0x202E
        $texts = @(('zq' + [char]0xD800), ([string][char]0xDC00 + [char]0xD800), ($rlo * 500), ('a' * 32000), ("line1`r`nline2`ttab"), '"', '|', '!', 'regex:(', 'regex:(a+)+$', 'size:>', 'ext:', 'dm:', ('😀' * 3000), ('ً' * 5000), ('ـ' * 5000), ('fuzzy:' + ('ab' * 2000)), ('root:' + ('ك' * 500)), '*', '\\?\', ([string][char]0xFFFF + [char]0xFFFE + [char]0))
        $sent = 0
        foreach ($t in $texts) {
            $u = Ui
            if (-not $u) { break }
            if ([TT]::SetText($u.Query.Handle, $t, 15000)) { $sent++ } else { [void](Check-Responsive 'hostile text in the search box') }
            Start-Sleep -Milliseconds 400
            if ([TM]::AppCount() -eq 0) { break }
        }
        Start-Sleep -Seconds 2
        $ok = Health 'hostile text in the search box'
        Record 'queries' 'core' 'hostile text in the search box leaves the program running and answering' ($ok -and $sent -eq $texts.Count) "$sent of $($texts.Count) texts taken"
        Gui-Clear
        # Three names typed with real keystrokes, as a person would.
        $typed = 0; $tries = 0
        $main = Show-Main
        if ($main) {
            foreach ($it in @($sample | Select-Object -First 3)) {
                $u = Ui -Fresh
                Focus-Window $u.Main.Handle
                [TT]::Click([int]($u.Query.X + $u.Query.W / 2), [int]($u.Query.Y + $u.Query.H / 2), $false)
                Start-Sleep -Milliseconds 200
                [TT]::Chord($VK.CTRL, $VK.A); [TT]::Chord($VK.BACK)
                [TT]::Type($it.Token)
                $tries++
                if (Wait-Until { List-Names 20 | Where-Object { $_ -and $_.ToLowerInvariant().Contains($it.Token) } | Select-Object -First 1 } 10) { $typed++ }
            }
        }
        Record 'names' 'probe' 'names typed with real keystrokes are found' ($tries -gt 0 -and $typed -eq $tries) "$typed of $tries"
        Gui-Clear
    }
    Step 'renames, deletes and new hostile names while the program watches' {
        if (-not $S.indexed) { return }
        $bytes = [Text.Encoding]::ASCII.GetBytes("late`r`n")
        $rand = @($S.items | Where-Object { $_.Kind -eq 'random' -and $_.Made -eq 1 })
        $expect = New-Object System.Collections.Generic.List[object]
        for ($i = 0; $i -lt 40 -and $i -lt $rand.Count; $i++) {
            $tok = New-Token
            $to = "$($rand[$i].Dir)\" + [TN]::RandomName(70000 + $i, $tok)
            if ([TF]::Move($rand[$i].Path, $to)) { $expect.Add(@{ Token = $rand[$i].Token; Present = $false; What = 'renamed away' }); $expect.Add(@{ Token = $tok; Present = $true; What = 'renamed to' }) }
        }
        for ($i = 40; $i -lt 80 -and $i -lt $rand.Count; $i++) {
            if ([TF]::Delete($rand[$i].Path)) { $expect.Add(@{ Token = $rand[$i].Token; Present = $false; What = 'deleted' }) }
        }
        $late = Join-Path $S.dir 'late'
        [void][TF]::MkDirs($late)
        foreach ($e in [TN]::Catalogue()) {
            if ($e[1] -eq 'dev' -or $e[1] -eq 'cs') { continue }
            $tok = New-Token
            if ([TF]::Write(("$late\" + $e[2].Replace('{T}', $tok)), $bytes, $false) -and $e[1] -ne 'lone') { $expect.Add(@{ Token = $tok; Present = $true; What = "created ($($e[0]))" }) }
        }
        # Lone surrogates in rename and delete notifications.
        $ls = "$late\late " + [char]0xD83D + ' lone.txt'
        [void][TF]::Write($ls, $bytes, $true)
        [void][TF]::Move($ls, ("$late\late " + [char]0xDE00 + ' moved.txt'))
        [void][TF]::Delete(("$late\late " + [char]0xDE00 + ' moved.txt'))
        Start-Sleep -Seconds 5
        [void](Health 'changes to hostile names')
        # The index is saved about 30 s after a change; after lost events the
        # program walks the drive again, which takes minutes.
        $limit = [Math]::Min(8, (Time-Left) - 2)
        $t0 = Get-Date; $wrong = @('not checked')
        while (((Get-Date) - $t0).TotalMinutes -lt $limit -and -not [TM]::Expired) {
            Start-Sleep -Seconds 20
            $map = Cli-Map $S.dir
            if ($null -eq $map) { $wrong = @("the command line failed: $script:cliError"); continue }
            $wrong = @($expect | Where-Object { $map.ContainsKey($_.Token) -ne $_.Present } | ForEach-Object { "$($_.What) [$($_.Token)]" })
            Detail ("after {0:N0} s: {1} of {2} changes not in the saved index yet" -f ((Get-Date) - $t0).TotalSeconds, $wrong.Count, $expect.Count)
            if ($wrong.Count -eq 0) { break }
        }
        $waited = ((Get-Date) - $t0).TotalSeconds
        $kind = if ($wrong.Count -eq 0 -or $waited -ge 240) { 'core' } else { 'probe' }
        Record 'names' $kind 'renames, deletes and creates of hostile names reach the saved index' ($wrong.Count -eq 0) ("{0} changes, waited {1:N0} s; wrong {2}: {3}; rescans: {4}" -f $expect.Count, $waited, $wrong.Count, (($wrong | Select-Object -First 6) -join '; '), ((Log-Lines 'rescan ') -join ' | '))
        [void](Health 'after changes to hostile names')
    } 3
}

# ================================================================ tree
function Group-Tree {
    $S.dir = Join-Path $Root 'tree'
    $S.big = Join-Path $Root 'big'
    $S.bigCount = 200000
    Step 'generate hostile folder shapes' {
        $t = $S.dir
        New-Item -ItemType Directory -Force $t | Out-Null
        $S.found = New-Object System.Collections.Generic.List[object]    # Token, What: must be found once
        $txt = [Text.Encoding]::ASCII.GetBytes("tree`r`n")
        function Must([string]$Token, [string]$What) { $S.found.Add([pscustomobject]@{ Token = $Token; What = $What }) }
        # 60 levels deep, a file every 10 levels.
        $d = Join-Path $t 'deep60'
        for ($i = 1; $i -le 60; $i++) {
            $d = Join-Path $d ('L{0:d2}' -f $i)
            [void][TF]::MkDirs($d)
            if ($i % 10 -eq 0) { $tok = New-Token; [void][TF]::Write("$d\$tok level $i.txt", $txt, $true); Must $tok "file $i levels deep" }
        }
        # Loops and links. Junctions and symbolic links are not followed by
        # the indexer; a file inside the loop must be listed exactly once.
        $loop = Join-Path $t 'loop'
        New-Item -ItemType Directory -Force (Join-Path $loop 'a\b'), (Join-Path $loop 'y'), (Join-Path $loop 'gone-target') | Out-Null
        $S.inLoop = New-Token
        [void][TF]::Write("$loop\a\$($S.inLoop) inloop.txt", $txt, $true)
        Must $S.inLoop 'file inside a junction loop'
        $links = 0
        foreach ($j in @(@('a\b\back', $loop), @('x', (Join-Path $loop 'y')), @('y\tox', (Join-Path $loop 'x')), @('dangling', (Join-Path $loop 'gone-target')), @('to-drive-root', "$env:SystemDrive\"), @('to-windows', $env:SystemRoot))) {
            try { New-Item -ItemType Junction -Path (Join-Path $loop $j[0]) -Target $j[1] -ErrorAction Stop | Out-Null; $links++ } catch { Detail "junction $($j[0]) failed: $_" }
        }
        Remove-Item -LiteralPath (Join-Path $loop 'gone-target') -Force -ErrorAction SilentlyContinue
        foreach ($sl in @(@('sym-self', '.', $true), @('sym-up', '..', $true), @('sym-file.txt', "a\$($S.inLoop) inloop.txt", $false), @('sym-dangling', 'Q:\nowhere\at-all', $true), @('sym-root', "$env:SystemDrive\", $true))) {
            if ([TF]::SymLink((Join-Path $loop $sl[0]), $sl[1], $sl[2])) { $links++ } else { Detail "symlink $($sl[0]) failed: error $([TF]::LastError)" }
        }
        Note 'tree' 'junctions and symbolic links' "$links made (loops, links to the drive root and to Windows, dangling ones)"
        # Hard links: one file under four names.
        $hl = Join-Path $t 'hardlinks'
        New-Item -ItemType Directory -Force $hl, (Join-Path $hl 'other') | Out-Null
        $tok = New-Token; $orig = "$hl\$tok original.txt"
        [void][TF]::Write($orig, $txt, $true); Must $tok 'hard link original'
        foreach ($n in 1..3) { $tok = New-Token; $to = if ($n -eq 3) { "$hl\other\$tok link.txt" } else { "$hl\$tok link $n.txt" }; if ([TF]::HardLink($to, $orig)) { Must $tok "hard link $n" } }
        # Sparse and empty files.
        $sp = Join-Path $t 'sparse'
        New-Item -ItemType Directory -Force $sp | Out-Null
        $S.sparseTb = New-Token
        if ([TF]::Sparse("$sp\$($S.sparseTb) sparse 1tb.bin", 1TB)) { Must $S.sparseTb 'sparse file of 1 TB' } else { Detail "sparse 1 TB failed: $([TF]::LastError)"; $S.sparseTb = $null }
        foreach ($spf in @(@('sparse 10gb.txt', 10GB), @('sparse 90mb.txt', 90MB), @('sparse 99mb.docx', 99MB), @('sparse 3gb.zip', 3GB))) { $tok = New-Token; if ([TF]::Sparse("$sp\$tok $($spf[0])", $spf[1])) { Must $tok $spf[0] } }
        foreach ($ext in 'txt', 'pdf', 'docx', 'xlsx', 'zip', 'jpg', 'exe', '') { $tok = New-Token; [void][TF]::Write("$sp\$tok empty.$ext".TrimEnd('.'), $null, $true); Must $tok "empty .$ext file" }
        # Odd timestamps (FILETIME values).
        $tm = Join-Path $t 'times'
        New-Item -ItemType Directory -Force $tm | Out-Null
        $epoch = 116444736000000000
        $S.times = New-Object System.Collections.Generic.List[object]
        $stamps = [ordered]@{ 'first tick of 1601' = 1; '1601-01-02' = 864000000000; 'one day before 1970' = $epoch - 864000000000; '1970 exactly' = $epoch; '2038-01-19 overflow' = $epoch + 21474836480000000;
            '2107 (FAT limit)' = ([datetime]'2107-12-31').ToFileTimeUtc(); 'year 9999' = [datetime]::SpecifyKind([datetime]::MaxValue, 'Utc').ToFileTimeUtc(); 'year 30827 (last valid)' = 0x7FFF35F4F06C0000; 'past the last valid' = 0x7FFFFFFFFFFFFFFF;
            '100 years ahead' = (Get-Date).AddYears(100).ToFileTimeUtc() }
        foreach ($k in $stamps.Keys) {
            $tok = New-Token; $p = "$tm\$tok time.txt"
            [void][TF]::Write($p, $txt, $true)
            $set = [TF]::SetTimes($p, [long]$stamps[$k])
            Must $tok "file dated $k"
            $S.times.Add([pscustomobject]@{ Token = $tok; Label = $k; Set = $set })
        }
        $tok = New-Token; $p = "$tm\$tok folder time"; [void][TF]::MkDir($p); [void][TF]::SetTimes($p, 1); Must $tok 'folder dated 1601'
        Note 'tree' 'timestamps' ("set: {0}; refused by Windows: {1}" -f (($S.times | Where-Object Set | ForEach-Object Label) -join ', '), (($S.times | Where-Object { -not $_.Set } | ForEach-Object Label) -join ', '))
        # Attributes.
        $at = Join-Path $t 'attributes'
        New-Item -ItemType Directory -Force $at | Out-Null
        foreach ($a in @(@('read-only', 1), @('hidden', 2), @('system', 4), @('hidden system', 6), @('read-only hidden system', 7), @('offline', 0x1000), @('temporary', 0x100), @('not content indexed', 0x2000))) {
            $tok = New-Token; $p = "$at\$tok $($a[0]).txt"; [void][TF]::Write($p, $txt, $true); [void][TF]::SetAttr($p, $a[1]); Must $tok "$($a[0]) file"
            $tok = New-Token; $p = "$at\$tok $($a[0]) folder"; [void][TF]::MkDir($p); [void][TF]::SetAttr($p, $a[1]); Must $tok "$($a[0]) folder"
        }
        # Access denied: a folder nobody may list, a file nobody may read.
        $dn = Join-Path $t 'denied'
        New-Item -ItemType Directory -Force $dn | Out-Null
        $S.deniedDir = New-Token; $S.deniedInside = New-Token
        $dd = "$dn\$($S.deniedDir) no entry"
        New-Item -ItemType Directory -Force $dd | Out-Null
        [void][TF]::Write("$dd\$($S.deniedInside) inside.txt", $txt, $true)
        if (Deny-Read $dd -Folder) { Must $S.deniedDir 'folder with access denied' } else { $S.deniedDir = $null }
        $tok = New-Token; $p = "$dn\$tok unreadable.txt"; [void][TF]::Write($p, $txt, $true)
        if (Deny-Read $p) { Must $tok 'file with read access denied' }
        # Files another process holds open exclusively (this test is that process).
        $lk = Join-Path $t 'locked'
        New-Item -ItemType Directory -Force $lk | Out-Null
        foreach ($ext in 'txt', 'docx', 'pdf', 'zip') {
            $tok = New-Token; $p = "$lk\$tok locked.$ext"; [void][TF]::Write($p, $txt, $true)
            try { $script:locks.Add([IO.File]::Open($p, 'Open', 'ReadWrite', 'None')); Must $tok "exclusively locked .$ext" } catch { Detail "lock failed: $_" }
        }
        Note 'tree' 'generated' "$($S.found.Count) special files and folders"
    }
    Step 'a folder with 200,000 files' {
        $free = Free-GB
        if ($free -ge 0 -and $free -lt 14) { $S.bigCount = 20000; Note 'disk' 'smaller folder' "only $free GB free: 20,000 files instead of 200,000" }
        New-Item -ItemType Directory -Force $S.big | Out-Null
        $made = Run-Job "create $($S.bigCount) files in one folder" 'many' $S.big 'zq' ([int[]]@(0x100000, $S.bigCount)) -NoApp
        $S.bigCount = $made
    }
    Step 'drives that come and go' {
        # Made before the first index, so the program indexes and watches them.
        $S.other = Second-Drive
        if ($S.other) {
            $S.otherDir = Join-Path $S.other 'ifx-other'
            New-Item -ItemType Directory -Force $S.otherDir | Out-Null
            $S.otherTok = New-Token
            [IO.File]::WriteAllText((Join-Path $S.otherDir "$($S.otherTok) on the second drive.txt"), 'x')
            try { New-Item -ItemType Junction -Path (Join-Path $S.dir 'loop\to-other-drive') -Target $S.other -ErrorAction Stop | Out-Null } catch { Detail "junction to $($S.other) failed: $_" }
        } else { Note 'drives' 'second drive' 'none: this runner has one fixed drive and a VHD could not be made' }
        # A scratch VHD as well when the second drive is a real one: it is
        # the drive that can be taken away while the program runs.
        if (-not $script:vhd) { [void](New-Vhd 300) }
        if ($script:vhd -and $S.other -ne "$($script:vhd.Letter):\") {
            $S.vhdTok = New-Token
            New-Item -ItemType Directory -Force "$($script:vhd.Letter):\ifx-other" | Out-Null
            [IO.File]::WriteAllText("$($script:vhd.Letter):\ifx-other\$($S.vhdTok) on the VHD.txt", 'x')
        }
        $target = Join-Path $S.dir 'subst-target'
        New-Item -ItemType Directory -Force $target | Out-Null
        $S.substTok = New-Token
        [IO.File]::WriteAllText((Join-Path $target "$($S.substTok) behind subst.txt"), 'x')
        $S.subst = New-Subst $target
        Note 'drives' 'set-up' "second drive: $($S.other); VHD: $(if ($script:vhd) { $script:vhd.Letter + ':' } else { 'none' }); subst: $(if ($S.subst) { $S.subst + ':' } else { 'none' })"
    }
    Step 'start and index' { [void](Fresh-Start) } 8

    Step 'command line finds the special files' {
        if (-not $S.indexed) { return }
        $map = Cli-Map $S.dir
        if ($null -eq $map) { Record 'tree' 'core' 'the command line lists the tree' $false $script:cliError; return }
        $bad = @()
        foreach ($f in $S.found) {
            $n = if ($map.ContainsKey($f.Token)) { $map[$f.Token].Count } else { 0 }
            if ($n -ne 1) { $bad += "$($f.What) [$($f.Token)] listed $n times" }
        }
        Record 'tree' 'core' 'every special file and folder is found exactly once' ($bad.Count -eq 0) ("{0} checked; wrong {1}: {2}" -f $S.found.Count, $bad.Count, (($bad | Select-Object -First 8) -join '; '))
        $inside = if ($map.ContainsKey($S.deniedInside)) { 'listed' } else { 'not listed (the folder cannot be opened)' }
        Note 'tree' 'file inside a folder with access denied' $inside
        # Junctions to the drive root and to Windows must not be followed.
        $r = Run-Cli @('-s', 'notepad.exe|explorer.exe|kernel32.dll', '--scope', (Join-Path $S.dir 'loop'), '--all', '--count')
        $n = -1; [void][int]::TryParse($r.Out.Trim(), [ref]$n)
        Record 'tree' 'core' 'junctions and symbolic links to the drive root are not followed' ($r.Code -eq 1 -or $n -eq 0) "exit $($r.Code), $n Windows files listed under the loop folder"
        $r = Run-Cli @('-s', 'zq', '--scope', (Join-Path $S.dir 'loop'), '--all', '--count')
        Note 'tree' 'entries under the loop folder' "$($r.Out.Trim()) with a token (expected 1)"
        # The folder of 200,000.
        $r = Run-Cli @('-s', 'zq1', '--scope', $S.big, '--all', '--count')
        $n = -1; [void][int]::TryParse($r.Out.Trim(), [ref]$n)
        Record 'tree' 'core' "all $($S.bigCount) files of the big folder are in the index" ($n -eq $S.bigCount) "$n counted in $($r.Ms) ms"
        $rng = New-Object Random 7
        $pick = @(1..40 | ForEach-Object { 'zq{0:x6}' -f (0x100000 + $rng.Next($S.bigCount)) } | Select-Object -Unique)
        $m2 = Cli-Tokens $pick $S.big
        $missing = @(if ($null -eq $m2) { $pick } else { $pick | Where-Object { -not $m2.ContainsKey($_) } })
        Record 'tree' 'core' 'a random sample of the big folder is found by name' ($missing.Count -eq 0) "$($pick.Count) sampled; missing: $($missing -join ' ')"
        # Sorting and formats over the odd dates and sizes.
        foreach ($a in @(@('--sort', 'date'), @('--sort', 'date', '--desc', '--csv'), @('--sort', 'size', '--desc', '--json'), @('--sort', 'size', '--csv'))) {
            $r = Run-Cli (@('-s', 'zq', '--scope', $S.dir, '--all') + $a)
            if ($r.Code -ne 0) { Record 'tree' 'core' "sorted output over odd dates and sizes ($($a -join ' '))" $false "exit $($r.Code)" }
            if ($a -contains '--csv' -and $a -contains 'date') { [IO.File]::WriteAllText((Join-Path $Out 'tree-dates.csv'), $r.Out, (New-Object Text.UTF8Encoding $true)) }
        }
        if ($S.sparseTb) {
            $r = Run-Cli @('-s', "$($S.sparseTb) size:>500gb", '--scope', $S.dir, '--all', '--count')
            Record 'tree' 'probe' 'size:>500gb finds the 1 TB sparse file' ($r.Out.Trim() -eq '1') "counted $($r.Out.Trim())"
        }
        if ($S.other) {
            $m3 = Cli-Tokens @($S.otherTok)
            $paths = @(if ($m3 -and $m3.ContainsKey($S.otherTok)) { $m3[$S.otherTok] })
            Record 'drives' 'core' 'a file on the second drive is in the index' ($paths.Count -eq 1 -and ([string]$paths[0]).StartsWith($S.other, [StringComparison]::OrdinalIgnoreCase)) (($paths | Select-Object -First 3) -join ' | ')
        }
        if ($S.vhdTok) {
            $m3 = Cli-Tokens @($S.vhdTok)
            $paths = @(if ($m3 -and $m3.ContainsKey($S.vhdTok)) { $m3[$S.vhdTok] })
            Record 'drives' 'core' 'a file on the scratch VHD is in the index' ($paths.Count -eq 1) (($paths | Select-Object -First 3) -join ' | ')
        }
        if ($S.subst) {
            $m3 = Cli-Tokens @($S.substTok)
            $paths = @(if ($m3 -and $m3.ContainsKey($S.substTok)) { $m3[$S.substTok] })
            Record 'drives' 'probe' 'a file behind a subst drive is listed under both letters' ($paths.Count -eq 2) (($paths | Select-Object -First 3) -join ' | ')
        }
        [void](Check-Crash 'command-line searches in the tree')
    }
    Step 'the window shows the special files' {
        if (-not $S.indexed) { return }
        [void](Show-Main)
        $miss = @(); $dates = @()
        foreach ($tm in $S.times) {
            $r = Gui-Find $tm.Token $tm.Token 12
            if (-not $r.Hit) { $miss += "$($tm.Label) ($($r.Status))"; continue }
            $u = Ui
            $date = [TT]::ListText($u.List.Handle, 0, 3)
            if ($null -eq $date) { $miss += "$($tm.Label): the date column did not answer" } else { $dates += "$($tm.Label) -> '$date'" }
        }
        Record 'tree' 'core' 'the window lists files with odd dates and shows a date for each' ($miss.Count -eq 0) (($miss -join '; ') + ' ' + ($dates -join '; '))
        if ($S.sparseTb) {
            $r = Gui-Find $S.sparseTb $S.sparseTb 12
            $u = Ui
            $size = if ($r.Hit) { [TT]::ListText($u.List.Handle, 0, 2) } else { '' }
            Record 'tree' 'core' 'the window lists the 1 TB sparse file' ($null -ne $r.Hit) "size shown: '$size'"
        }
        # 200,000 rows in the list.
        $u = Ui
        $t = Get-Date
        [void][TT]::SetText($u.Query.Handle, 'zq1', 15000)
        $n = Wait-Until { $c = [TT]::ListCount($u.List.Handle); if ($c -ge $S.bigCount) { $c } } 60 500
        $secs = ((Get-Date) - $t).TotalSeconds
        Record 'tree' 'core' "the window lists all $($S.bigCount) files of the big folder" ($null -ne $n) ("{0} rows after {1:N1} s; status: {2}" -f [TT]::ListCount($u.List.Handle), $secs, (Short (Status-Text) 120))
        # Rows from the far end, where a virtual list asks for them one by one.
        foreach ($row in 0, 1000, 99999, ($S.bigCount - 1)) { if ($row -lt $S.bigCount) { [void][TT]::ListText($u.List.Handle, $row, 0); [void][TT]::ListText($u.List.Handle, $row, 1) } }
        Shot 'tree-200k-rows'
        [void](Health 'the window with 200,000 rows')
        Gui-Clear
        [void](Gui-Seed 'after the big list')
    }
    Step 'the second drive goes away and comes back' {
        if (-not $S.indexed) { return }
        if (-not $script:vhd -and -not $S.subst) { Note 'drives' 'skipped' 'no removable scratch drive on this runner'; return }
        $gone = @()
        if ($script:vhd) { if (Detach-Vhd) { $gone += "VHD $($script:vhd.Letter):" } }
        if ($S.subst) { Remove-Subst; $gone += "subst $($S.subst):" }
        Note 'drives' 'removed while the program runs' ($gone -join ', ')
        Start-Sleep -Seconds 20
        $ok = Health 'after a drive was removed'
        Record 'drives' 'core' 'the program keeps running and answering after a drive is removed' $ok (((Log-Lines 'watch|rescan') | Select-Object -Last 3) -join ' | ')
        [void](Gui-Seed 'after a drive was removed')
        # Back again, with a new file on each.
        $new = @{}
        if ($script:vhd -and (Attach-Vhd)) {
            $tok = New-Token
            try { New-Item -ItemType Directory -Force "$($script:vhd.Letter):\ifx-other" | Out-Null; [IO.File]::WriteAllText("$($script:vhd.Letter):\ifx-other\$tok after coming back.txt", 'x'); $new[$tok] = "VHD $($script:vhd.Letter):" } catch { Detail "write to the VHD failed: $_" }
        }
        if ($S.subst) {
            $r = [TT]::Run('subst.exe', @("$($S.subst):", (Join-Path $S.dir 'subst-target')), 20000)
            $tok = New-Token
            try { [IO.File]::WriteAllText("$($S.subst):\$tok after coming back.txt", 'x'); $new[$tok] = "subst $($S.subst):" } catch { Detail "write to the subst drive failed: $_" }
        }
        Start-Sleep -Seconds 10
        $ok = Health 'after the drive came back'
        Record 'drives' 'core' 'the program keeps running and answering after the drive comes back' $ok ''
        if ($new.Count) {
            $found = @{}
            $t0 = Get-Date
            while (((Get-Date) - $t0).TotalSeconds -lt 100 -and -not (Out-Of-Time) -and $found.Count -lt $new.Count) {
                Start-Sleep -Seconds 20
                $m = Cli-Tokens @($new.Keys)
                if ($m) { foreach ($k in $m.Keys) { $found[$k] = $true } }
            }
            $missing = @($new.Keys | Where-Object { -not $found.ContainsKey($_) } | ForEach-Object { $new[$_] })
            Record 'drives' 'probe' 'a file made on a drive that came back is picked up within 100 s' ($missing.Count -eq 0) "not picked up on: $($missing -join ', ')"
        }
    } 3
    Step 'delete the 200,000 files while the program watches' {
        if (-not $S.indexed) { return }
        Release-Locks
        Undo-Acls
        $failed = Run-Job 'delete the big folder under the watcher' 'deltree' $S.big '' ([int[]]@())
        Start-Sleep -Seconds 10
        $ok = Health 'after deleting 200,000 files'
        Record 'tree' 'core' 'the program survives 200,000 deletions in one folder' $ok "entries that could not be deleted: $failed"
        [void](Gui-Seed 'after deleting 200,000 files')
    } 2
}

# ================================================================ churn
# Checks sampled expectations against the saved index. Each: @{ Token;
# Present; Under (text the path must hold, or '') }. Returns what is wrong.
function Verify-Expected($Expect) {
    $map = Cli-Tokens @($Expect | ForEach-Object { $_.Token })
    if ($null -eq $map) { return @("the command line failed: $script:cliError") }
    $wrong = @()
    foreach ($e in $Expect) {
        $paths = @(if ($map.ContainsKey($e.Token)) { $map[$e.Token] })
        if (-not $e.Present) { if ($paths.Count) { $wrong += "$($e.What) [$($e.Token)] still listed" }; continue }
        if ($paths.Count -eq 0) { $wrong += "$($e.What) [$($e.Token)] not found" }
        elseif ($paths.Count -gt 1) { $wrong += "$($e.What) [$($e.Token)] listed $($paths.Count) times" }
        elseif ($e.Under -and ([string]$paths[0]).IndexOf($e.Under, [StringComparison]::OrdinalIgnoreCase) -lt 0) { $wrong += "$($e.What) [$($e.Token)] listed at the old place" }
    }
    return $wrong
}
# Waits until the saved index matches `Expect`, at most `MaxMinutes` (a
# storm that overflows the watcher costs a walk of the whole drive, and the
# program spaces those 10 minutes apart).
function Wait-Converged($Expect, [double]$MaxMinutes, [string]$Area, [string]$Check) {
    $limit = [Math]::Min($MaxMinutes, (Time-Left) - 2.5)
    $t0 = Get-Date; $wrong = @('not checked'); $rounds = 0
    while (-not [TM]::Expired) {
        $wrong = @(Verify-Expected $Expect)
        $rounds++
        $mins = ((Get-Date) - $t0).TotalMinutes
        Say ("  {0:N1} min after the changes: {1} of {2} sampled files differ from the disk" -f $mins, $wrong.Count, $Expect.Count)
        if ($wrong.Count -eq 0 -or $mins -ge $limit) { break }
        if (-not (Health 'while the index settles')) { break }
        Start-Sleep -Seconds 30
    }
    $mins = ((Get-Date) - $t0).TotalMinutes
    # Only a full wait makes it a failure; a wait cut short by the time
    # budget is reported as a finding.
    $kind = if ($wrong.Count -eq 0 -or $mins -ge $MaxMinutes - 0.5) { 'core' } else { 'probe' }
    Record $Area $kind $Check ($wrong.Count -eq 0) ("{0} files sampled, waited {1:N1} min; wrong {2}: {3}; rescans: {4}" -f $Expect.Count, $mins, $wrong.Count, (($wrong | Select-Object -First 8) -join '; '), (((Log-Lines 'rescan ') | Select-Object -Last 4) -join ' | '))
    if ($wrong.Count) { [IO.File]::WriteAllLines((Join-Path $Out 'not-converged.txt'), [string[]]$wrong, (New-Object Text.UTF8Encoding $true)) }
    return ($wrong.Count -eq 0)
}
function Group-Churn {
    $S.dir = Join-Path $Root 'churn'
    # Token ranges: A the storm's files, B what they are renamed to, F the
    # folder that is renamed back and forth, D the tree that is deleted,
    # M the tree that moves to another drive.
    $S.A = 0x200000; $S.B = 0x300000; $S.F = 0x400000; $S.D = 0x500000; $S.M = 0x700000
    $S.nA = 30000; $S.nF = 5000; $S.nD = 100000; $S.nM = 2000; $S.per = 500
    Step 'generate the trees' {
        $free = Free-GB
        if ($free -ge 0 -and $free -lt 14) { $S.nD = 10000; $S.nA = 6000; Note 'disk' 'smaller storm' "only $free GB free" }
        New-Item -ItemType Directory -Force $S.dir | Out-Null
        [void](Run-Job "tree to delete later ($($S.nD) files)" 'create' (Join-Path $S.dir 'doomed') '' ([int[]]@($S.D, $S.nD, $S.per)) -NoApp)
        [void](Run-Job "folder to rename back and forth ($($S.nF) files)" 'create' (Join-Path $S.dir 'flipA') '' ([int[]]@($S.F, $S.nF, $S.per)) -NoApp)
        [void](Run-Job "tree to move across drives ($($S.nM) files)" 'create' (Join-Path $S.dir 'mv') '' ([int[]]@($S.M, $S.nM, $S.per)) -NoApp)
        $S.other = Second-Drive
        Note 'drives' 'second drive for moves' $(if ($S.other) { $S.other } else { 'none: moving across drives is skipped' })
    }
    # In the tray, as after signing in.
    Step 'start in the tray and index' { [void](Fresh-Start @('--startup')) } 8

    Step 'storms' {
        if (-not $S.indexed) { Note 'churn' 'skipped' 'no index'; return }
        $st = Join-Path $S.dir 'st'
        [void](Run-Job "storm: create $($S.nA) files" 'create' $st '' ([int[]]@($S.A, $S.nA, $S.per)))
        [void](Run-Job 'storm: rename every third file' 'rename' $st '' ([int[]]@($S.A, $S.nA, $S.per, 3, 0, $S.B)))
        [void](Run-Job 'storm: delete every third file' 'delete' $st '' ([int[]]@($S.A, $S.nA, $S.per, 3, 1)))
        [void](Run-Job 'storm: replace files by files of the same name' 'replace' $st '' ([int[]]@($S.A, $S.nA, $S.per, 6, 2)))
        [void](Run-Job 'storm: rename files to capitals only' 'upper' $st '' ([int[]]@($S.A, $S.nA, $S.per, 6, 5)))
        $S.flips = Run-Job 'storm: rename a folder of 5,000 files back and forth 21 times' 'flip' (Join-Path $S.dir 'flipA') (Join-Path $S.dir 'flipB') ([int[]]@(21, 300))
        $S.doomedLeft = Run-Job "storm: delete the tree of $($S.nD) files" 'deltree' (Join-Path $S.dir 'doomed') '' ([int[]]@())
        if ($S.other) {
            # Across drives a move is a copy and a delete; robocopy does both.
            $dst = Join-Path $S.other 'ifx-mv'
            $r = [TT]::Run('robocopy.exe', @((Join-Path $S.dir 'mv'), $dst, '/E', '/MOVE', '/NFL', '/NDL', '/NJH', '/NJS', '/NP', '/R:1', '/W:1'), 240000)
            $S.movedOut = ($r.Code -ge 0 -and $r.Code -lt 8)
            Note 'files' 'storm: move a tree to the second drive' "robocopy exit $($r.Code) in $($r.Ms) ms"
            # ... and the first folder of it back again.
            $r = [TT]::Run('robocopy.exe', @((Join-Path $dst 'd000'), (Join-Path $S.dir 'mvback\d000'), '/E', '/MOVE', '/NFL', '/NDL', '/NJH', '/NJS', '/NP', '/R:1', '/W:1'), 240000)
            $S.movedBack = ($r.Code -ge 0 -and $r.Code -lt 8)
            Note 'files' 'storm: move one folder back' "robocopy exit $($r.Code) in $($r.Ms) ms"
            [void](Health 'moves across drives')
        }
        Start-Sleep -Seconds 5
        $ok = Health 'after the storms'
        Record 'churn' 'core' 'the program is running and answering after the storms' $ok ("slowest answer {0} ms" -f $script:pingMax)
        Keep-Log 'after-storms'
    } 6
    Step 'the index matches the disk' {
        if (-not $S.indexed) { return }
        $rng = New-Object Random 11
        $expect = New-Object System.Collections.Generic.List[object]
        function Tok([int]$n) { return 'zq{0:x6}' -f $n }
        $st = '\churn\st\'
        for ($k = 0; $k -lt 150; $k++) {
            $i = $rng.Next($S.nA)
            switch ($i % 3) {
                0 { $expect.Add(@{ Token = (Tok ($S.A + $i)); Present = $false; Under = ''; What = 'renamed away' }); $expect.Add(@{ Token = (Tok ($S.B + $i)); Present = $true; Under = $st; What = 'renamed to' }) }
                1 { $expect.Add(@{ Token = (Tok ($S.A + $i)); Present = $false; Under = ''; What = 'deleted' }) }
                2 { $expect.Add(@{ Token = (Tok ($S.A + $i)); Present = $true; Under = $st; What = $(if ($i % 6 -eq 2) { 'replaced by a file of the same name' } else { 'renamed to capitals' }) }) }
            }
        }
        $flipTo = if ($S.flips % 2 -eq 1) { '\churn\flipB\' } else { '\churn\flipA\' }
        for ($k = 0; $k -lt 40; $k++) { $expect.Add(@{ Token = (Tok ($S.F + $rng.Next($S.nF))); Present = $true; Under = $flipTo; What = "in the folder renamed $($S.flips) times" }) }
        for ($k = 0; $k -lt 60; $k++) { $expect.Add(@{ Token = (Tok ($S.D + $rng.Next($S.nD))); Present = $false; Under = ''; What = 'in the deleted tree' }) }
        if ($S.other -and $S.movedOut) {
            for ($k = 0; $k -lt 30; $k++) {
                $i = $rng.Next($S.nM)
                $under = if ($i -lt $S.per -and $S.movedBack) { '\churn\mvback\' } else { (Join-Path $S.other 'ifx-mv') + '\' }
                $expect.Add(@{ Token = (Tok ($S.M + $i)); Present = $true; Under = $under; What = 'moved across drives' })
            }
        }
        # No token twice (the random picks can repeat).
        $seen = @{}; $unique = @($expect | Where-Object { if ($seen.ContainsKey($_.Token)) { $false } else { $seen[$_.Token] = 1; $true } })
        [void](Wait-Converged $unique 16 'churn' 'after the storms settle, the saved index matches the disk (random sample)')
        [void](Health 'after the index settled')
        # The window's own index (in memory) as well: one file that exists, one that is gone.
        $present = $unique | Where-Object { $_.Present } | Select-Object -First 1
        $absent = $unique | Where-Object { -not $_.Present } | Select-Object -First 1
        if ($present) { $r = Gui-Find $present.Token $present.Token 20; Record 'churn' 'probe' 'the hidden window finds a file made in the storm' ($null -ne $r.Hit) "$($present.What) [$($present.Token)]: $($r.Status)" }
        if ($absent) {
            $r = Gui-Find $absent.Token $absent.Token 5
            Record 'churn' 'probe' 'the hidden window no longer lists a deleted file' ($null -eq $r.Hit) "$($absent.What) [$($absent.Token)]"
        }
        Gui-Clear
    } 3
}

# ================================================================ state
# Restores the files of the data folder that a scenario may have damaged
# from the copies taken while everything was good.
function Restore-Data {
    Release-Locks
    foreach ($name in 'index.bin', 'settings.txt', 'history.json', 'log.txt', 'crash.txt', 'index.tmp', 'settings.txt.tmp', 'content') {
        $p = Join-Path $data $name
        if (-not [TF]::Exists($p)) { continue }
        $item = Get-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue
        if (-not $item) { continue }
        if ($item.Attributes -band [IO.FileAttributes]::ReadOnly) { $item.Attributes = 'Normal' }
        $good = Join-Path $S.good $name
        $isDir = [bool]($item.Attributes -band [IO.FileAttributes]::Directory)
        # A folder where a file belongs, or a file where the folder belongs.
        if ($isDir -and $name -ne 'content') { [void][TF]::DeleteTree($p) }
        if (-not $isDir -and $name -eq 'content') { Remove-Item -LiteralPath $p -Force }
    }
    foreach ($name in 'index.bin', 'settings.txt', 'history.json') {
        $good = Join-Path $S.good $name
        $p = Join-Path $data $name
        if (Test-Path -LiteralPath $good) {
            if (-not (Test-Path -LiteralPath $p) -or (Get-Item -LiteralPath $p).Length -ne (Get-Item -LiteralPath $good).Length -or $S.dirty -contains $name) { Copy-Item -LiteralPath $good -Destination $p -Force }
        }
    }
    foreach ($name in 'index.tmp', 'settings.txt.tmp', 'crash.txt') { $p = Join-Path $data $name; if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue } }
    if ($S.dirty -contains 'content' -and (Test-Path -LiteralPath (Join-Path $S.good 'content'))) {
        if (Test-Path -LiteralPath $contentDir) { [void][TF]::DeleteTree($contentDir) }
        Copy-Item -LiteralPath (Join-Path $S.good 'content') -Destination $contentDir -Recurse -Force
    }
    $S.dirty = @()
}
function Fill-File([string]$Path, [long]$Length, [string]$With) {
    $fs = [IO.File]::Open($Path, 'Create', 'Write', 'None')
    try {
        $block = New-Object byte[] (1MB)
        $rng = New-Object Random 99
        for ([int64]$done = 0; $done -lt $Length; $done += $block.Length) {
            if ($With -eq 'random') { $rng.NextBytes($block) }
            $fs.Write($block, 0, [int][Math]::Min([int64]$block.Length, [int64]($Length - $done)))
        }
    } finally { $fs.Dispose() }
}
function Cut-File([string]$Path, [long]$Length) {
    $fs = [IO.File]::Open($Path, 'Open', 'ReadWrite', 'None')
    try { $fs.SetLength([Math]::Max([int64]0, [Math]::Min([int64]$fs.Length, [int64]$Length))) } finally { $fs.Dispose() }
}
function Patch-File([string]$Path, [long]$Offset, [byte[]]$Bytes) {
    $fs = [IO.File]::Open($Path, 'Open', 'ReadWrite', 'None')
    try { if ($fs.Length -ge $Offset + $Bytes.Length) { [void]$fs.Seek($Offset, 'Begin'); $fs.Write($Bytes, 0, $Bytes.Length) } } finally { $fs.Dispose() }
}
function File-To-Folder([string]$Path) {
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force -Recurse }
    New-Item -ItemType Directory -Force $Path | Out-Null
    [IO.File]::WriteAllText((Join-Path $Path 'inside.txt'), 'a folder where a file belongs')
}
# One damaged-state start: damage, start, watch. Core: the window comes,
# the program stays and answers, no crash report. With -Find also: the
# window still finds a seed file (the index itself was not touched).
function Scenario([string]$Area, [string]$Label, [string[]]$Dirty, [scriptblock]$Damage, [int]$Observe = 6, [switch]$Find, [string]$LogPattern = '') {
    if (Out-Of-Time) { $S.skipped++; return }
    [void](Stop-App)
    Restore-Data
    $S.dirty = $Dirty
    try { & $Damage } catch {
        $msg = "$($_.Exception.Message) (line $($_.InvocationInfo.ScriptLineNumber))"
        if ($msg -match 'Could not find|Cannot find path|does not exist') { Note $Area "skipped: $Label" "the file to damage is not there: $msg"; $S.skipped++ }
        else { Record 'harness' 'core' "the damage for '$Label' could be made" $false $msg }
        return
    }
    [void](Start-App)
    $main = Wait-Until { Main-Window } 30
    $end = (Get-Date).AddSeconds($Observe)
    do { Start-Sleep -Milliseconds 700; [void](Dismiss-Dialogs $Label) } while ((Get-Date) -lt $end)
    $alive = ([TM]::AppCount() -ge 1)
    $ping = if ($main -and $alive) { [TT]::Ping($main.Handle, 15000) } else { -1 }
    if ($S.planted) {
        # The garbage crash.txt this scenario put there: unchanged means no
        # new report was added to it.
        $S.planted = $false
        try { if ((Test-Path -LiteralPath $crash) -and (Get-FileHash -LiteralPath $crash -Algorithm SHA256).Hash -eq $S.plantedHash) { Remove-Item -LiteralPath $crash -Force } } catch { Detail "planted crash.txt: $_" }
    }
    $crashOk = Check-Crash "${Area}: $Label"
    $log = if ($LogPattern) { (Log-Lines $LogPattern | Select-Object -Last 2) -join ' | ' } else { '' }
    $ok = ($null -ne $main) -and $alive -and ($ping -ge 0) -and $crashOk
    Record $Area 'core' "starts and stays up with $Label" $ok ("window {0}, running {1}, answered in {2} ms{3}" -f $(if ($main) { 'yes' } else { 'NO' }), $alive, $ping, $(if ($log) { "; log: $log" } else { '' }))
    if (-not $ok) { Shot "state-$Label"; Keep-Log "state-$($Label -replace '[^A-Za-z0-9]+', '-')" }
    if ($Find -and $ok) { [void](Gui-Seed $Label) }
    $S.scenarios++
}
function Group-State {
    $S.good = Join-Path $Scratch 'good'
    $S.dirty = @(); $S.skipped = 0; $S.scenarios = 0
    Step 'kill during the first indexing (5 times)' {
        Reset-AppData
        Write-Settings
        $rng = New-Object Random 5
        $bad = @()
        for ($i = 1; $i -le 5; $i++) {
            [void](Start-App)
            $wait = 2 + $rng.Next(18)
            Start-Sleep -Seconds $wait
            $was = [TM]::AppCount()
            $gone = Stop-App
            if ($was -ne 1) { $bad += "kill ${i}: $was processes were running before the kill" }
            if (-not $gone) { $bad += "kill ${i}: a process is left after the kill" }
            if (-not (Check-Crash "kill $i during the first indexing")) { $bad += "kill ${i}: crash report" }
            # Only a complete index may ever be saved.
            if (Test-Path -LiteralPath $index) { if (-not (Check-Seeds "index.bin found after kill $i, $wait s into the first indexing")) { $bad += "kill ${i}: an incomplete index was saved" } }
            Detail "kill $i after $wait s: index.bin $(if (Test-Path -LiteralPath $index) { 'exists' } else { 'not there yet' }), index.tmp $(Test-Path -LiteralPath $indexTmp)"
        }
        Record 'kill' 'core' 'killed 5 times during the first indexing: no crash report, no process left, no partial index' ($bad.Count -eq 0) ($bad -join '; ')
    } 4
    Step 'first index' {
        [void](Start-App)
        [void](Wait-Main 'after the kills')
        $S.indexed = Wait-Index 20
        [void](Health 'first index after the kills')
        if (-not $S.indexed) { return }
        [void](Check-Seeds 'first index after 5 kills')
        [void](Gui-Seed 'first index after 5 kills')
        # Let the content index store something, then keep copies of the good state.
        Start-Sleep -Seconds 25
        [void](Stop-App)
        New-Item -ItemType Directory -Force $S.good | Out-Null
        foreach ($name in 'index.bin', 'settings.txt', 'history.json') { $p = Join-Path $data $name; if (Test-Path -LiteralPath $p) { Copy-Item -LiteralPath $p -Destination (Join-Path $S.good $name) -Force } }
        if (Test-Path -LiteralPath $contentDir) {
            $size = (Get-ChildItem -LiteralPath $contentDir -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
            if ($size -lt 400MB) { Copy-Item -LiteralPath $contentDir -Destination (Join-Path $S.good 'content') -Recurse -Force }
            Note 'state' 'content index after the first minutes' ("{0:N1} MB in {1}" -f ($size / 1MB), ((Get-ChildItem -LiteralPath $contentDir -File -ErrorAction SilentlyContinue | ForEach-Object Name) -join ', '))
        }
        Note 'state' 'files in the data folder' ((Get-ChildItem -LiteralPath $data -Force | ForEach-Object { "$($_.Name) ($([Math]::Round($_.Length / 1KB)) KB)" }) -join ', ')
    } 8
    Step 'kill during an index save (4 times)' {
        if (-not $S.indexed) { return }
        $bad = @(); $hit = 0
        for ($i = 1; $i -le 4; $i++) {
            if ((Time-Left) -lt 12) { Note 'time' 'kills during a save' "stopped after $($i - 1) of 4"; break }
            [void](Start-App)
            if (-not (Wait-Loaded 60)) { $bad += "start ${i}: the index did not load in 60 s"; [void](Stop-App); continue }
            [void](Wait-Until { Log-Lines '^\d+ watch:' } 20)
            Start-Sleep -Seconds 2
            # A change on disk; the program saves its index about 30 s later,
            # writing index.tmp first. Kill it at that moment.
            [IO.File]::WriteAllText((Join-Path $Root "seed\late-$i-$(New-Token).txt"), 'x')
            $ms = [TM]::KillWhenFileAppears($indexTmp, 60000)
            if ($ms -ge 0) { $hit++ } else { [void][TM]::KillApp() }
            $gone = Wait-Until { [TM]::AppCount() -eq 0 } 10 100
            if (-not $gone) { $bad += "kill ${i}: a process is left" }
            if (-not (Check-Crash "kill $i during an index save")) { $bad += "kill ${i}: crash report" }
            Detail "kill ${i}: index.tmp appeared after $ms ms; left behind: $(Test-Path -LiteralPath $indexTmp)"
            if (-not (Check-Seeds "after kill $i during an index save")) { $bad += "kill ${i}: the saved index is broken" }
        }
        Record 'kill' 'core' 'killed while the index was being saved: the saved index stays whole' ($bad.Count -eq 0) "$hit kills landed during a save; $($bad -join '; ')"
        if ($hit -eq 0) { Note 'kill' 'no save seen' 'index.tmp never appeared within 60 s of a change: the kills did not land in a save' }
    } 5
    Step 'kill while documents are read (6 times)' {
        if (-not $S.indexed) { return }
        $bad = @(); $opens = @()
        $rng = New-Object Random 6
        for ($i = 1; $i -le 6; $i++) {
            if ((Time-Left) -lt 10) { Note 'time' 'kills during content indexing' "stopped after $($i - 1) of 6"; break }
            [void](Start-App)
            # The content index starts 5 s after the index has loaded.
            Start-Sleep -Seconds (7 + $rng.Next(10))
            $open = (Log-Lines 'content index open') | Select-Object -Last 1
            if ($open) { $opens += ($open -replace '^\d+ ', '') }
            if ($open -match 'open failed') { $bad += "start ${i}: $open" }
            $gone = Stop-App
            if (-not $gone) { $bad += "kill ${i}: a process is left" }
            if (-not (Check-Crash "kill $i while documents are read")) { $bad += "kill ${i}: crash report" }
        }
        [void](Start-App)
        # After a kill the start scans the drives again, and the content
        # index opens 5 s after that scan ends (10 s or more after the start).
        $open = Wait-Until { (Log-Lines 'content index open') | Select-Object -Last 1 } 60 500
        Record 'kill' 'core' 'killed while documents were being read: the content index opens again every time' ($bad.Count -eq 0 -and $open -and $open -notmatch 'open failed') ("last: {0}; all: {1}; {2}" -f $open, (($opens | Select-Object -Unique | Select-Object -First 4) -join ' / '), ($bad -join '; '))
        [void](Gui-Seed 'after the kills while documents were read')
        [void](Check-Seeds 'after 15 kills')
        [void](Stop-App)
    } 4
    # The damaged-state starts come in this order so that a run that is
    # short of time has done the files that matter most.
    Step 'damaged settings.txt' {
        if (-not $S.indexed) { return }
        $f = $settingsFile
        Scenario 'settings' 'an empty settings.txt' @('settings.txt') { [IO.File]::WriteAllBytes($f, (New-Object byte[] 0)) } 5 -Find
        Scenario 'settings' 'settings.txt of 4 KB of zeros' @('settings.txt') { Fill-File $f 4096 'zero' } 5 -Find
        Scenario 'settings' 'settings.txt of random bytes' @('settings.txt') { Fill-File $f 8192 'random' } 5 -Find
        Scenario 'settings' 'settings.txt of 27 MB of garbage lines' @('settings.txt') { [IO.File]::WriteAllText($f, ((("width=" + ('9' * 4000) + "`ncolumns=" + ('-1,' * 2000) + "`n=`n====`nkind=x`n") * 2700))) } 5 -Find
        Scenario 'settings' 'absurd values in settings.txt' @('settings.txt') {
            [IO.File]::WriteAllText($f, "width=-5`nheight=99999999999999999999`nmaximized=2`ncolumns=-1,0,99999999,x,,2147483647,-2147483648`narabic_digits=yes`nkind=4294967295`nsort_col=77`nsort_desc=1`nupdate_check=18446744073709551615`ncrash_seen=-1`nfloating_mode=sideways`nfloating_pos=1,2,3`nhotkey_opens=nothing`nfile_opens=-7`nfloating_asked=1`nmove_asked=1`nrating_asked=1`nnew_ui=0`nnew_ui_default=1`n")
        } 5 -Find
        Scenario 'settings' 'a floating box position far off the screen' @('settings.txt') { [IO.File]::WriteAllText($f, "floating_asked=1`nmove_asked=1`nrating_asked=1`nfloating_mode=fade`nfloating_pos=0,0,1024,768,-2000000000,2000000000`nwidth=10000`nheight=10000`nmaximized=1`n") } 5 -Find
        Scenario 'settings' 'settings.txt in UTF-16' @('settings.txt') { [IO.File]::WriteAllText($f, "floating_asked=1`nmove_asked=1`nwidth=900`n", [Text.Encoding]::Unicode) } 5 -Find
        Scenario 'settings' 'a read-only settings.txt' @('settings.txt') { (Get-Item -LiteralPath $f).Attributes = 'ReadOnly' } 5 -Find
        Scenario 'settings' 'a folder named settings.txt' @('settings.txt') { File-To-Folder $f } 5 -Find
        Scenario 'settings' 'settings.txt held open by another program' @('settings.txt') { $script:locks.Add([IO.File]::Open($f, 'Open', 'ReadWrite', 'None')) } 5 -Find
    } 2
    Step 'damaged index.bin' {
        if (-not $S.indexed) { return }
        $f = $index
        $len = (Get-Item -LiteralPath $f).Length
        $lp = 'index load'
        Scenario 'index' 'index.bin cut to 3 bytes' @('index.bin') { Cut-File $f 3 } 6 -LogPattern $lp
        Scenario 'index' 'index.bin cut in the middle' @('index.bin') { Cut-File $f ([long]($len * 0.47)) } 7 -LogPattern $lp
        Scenario 'index' 'index.bin one byte short' @('index.bin') { Cut-File $f ($len - 1) } 7 -LogPattern $lp
        Scenario 'index' 'index.bin of zeros' @('index.bin') { Fill-File $f $len 'zero' } 7 -LogPattern $lp
        Scenario 'index' 'an empty index.bin' @('index.bin') { [IO.File]::WriteAllBytes($f, (New-Object byte[] 0)) } 6 -LogPattern $lp
        Scenario 'index' 'index.bin with another version number' @('index.bin') { Patch-File $f 0 ([byte[]](2, 0, 0, 0)) } 7 -LogPattern $lp
        Scenario 'index' 'index.bin claiming 2^63 entries' @('index.bin') { Patch-File $f 4 ([byte[]](0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F)) } 7 -LogPattern $lp
        Scenario 'index' 'index.bin with a string of 2^62 bytes' @('index.bin') { Patch-File $f 12 ([byte[]](0, 0, 0, 0, 0, 0, 0, 0x40)) } 7 -LogPattern $lp
        Scenario 'index' 'index.bin of 300 MB of garbage' @('index.bin') { if (Have-Space 1 '300 MB of garbage') { Fill-File $f 300MB 'random' } } 7 -LogPattern $lp
        Scenario 'index' 'a folder named index.bin' @('index.bin') { File-To-Folder $f } 6 -LogPattern $lp
        Scenario 'index' 'index.bin held open by another program' @('index.bin') { $script:locks.Add([IO.File]::Open($f, 'Open', 'ReadWrite', 'None')) } 7 -LogPattern $lp
        Release-Locks
        Scenario 'index' 'a good but read-only index.bin' @('index.bin') { (Get-Item -LiteralPath $f).Attributes = 'ReadOnly' } 6 -Find -LogPattern $lp
        Scenario 'index' 'a folder named index.tmp (nothing can be saved)' @('index.tmp') { File-To-Folder $indexTmp } 6 -Find -LogPattern $lp
        Scenario 'index' 'left-over index.tmp of garbage' @('index.tmp') { Fill-File $indexTmp 5MB 'random' } 6 -Find -LogPattern $lp
    } 2
    Step 'damaged content index' {
        if (-not $S.indexed) { return }
        $dat = Join-Path $contentDir 'content.dat'; $idx = Join-Path $contentDir 'content.idx'
        if (-not (Test-Path -LiteralPath $dat) -or -not (Test-Path -LiteralPath (Join-Path $S.good 'content'))) { Note 'content index' 'skipped' "no content.dat to damage (or the store is too big to restore): $((Get-ChildItem -LiteralPath $contentDir -ErrorAction SilentlyContinue | ForEach-Object Name) -join ', ')"; return }
        # (The content index is opened 5 s after the file index has loaded.)
        $lp = 'content index open'
        foreach ($target in @(@('content.dat', $dat), @('content.idx', $idx))) {
            $n = $target[0]; $p = $target[1]
            if (-not (Test-Path -LiteralPath $p)) { continue }
            Scenario 'content index' "$n cut to 3 bytes" @('content') { Cut-File $p 3 } 9 -Find -LogPattern $lp
            Scenario 'content index' "$n cut in the middle" @('content') { Cut-File $p ([long]((Get-Item -LiteralPath $p).Length * 0.61)) } 9 -Find -LogPattern $lp
            Scenario 'content index' "$n all zeros" @('content') { Fill-File $p (Get-Item -LiteralPath $p).Length 'zero' } 9 -Find -LogPattern $lp
            Scenario 'content index' "$n random bytes" @('content') { Fill-File $p ([Math]::Min([int64](Get-Item -LiteralPath $p).Length, [int64]50MB)) 'random' } 9 -Find -LogPattern $lp
            Scenario 'content index' "$n with another version and huge lengths in its header" @('content') { Patch-File $p 8 ([byte[]](0xE7, 0x03, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF, 0x7F)) } 9 -Find -LogPattern $lp
            Scenario 'content index' "a folder named $n" @('content') { File-To-Folder $p } 9 -Find -LogPattern $lp
            Scenario 'content index' "$n held open by another program" @('content') { $script:locks.Add([IO.File]::Open($p, 'Open', 'ReadWrite', 'None')) } 9 -Find -LogPattern $lp
        }
        Scenario 'content index' 'content.dat without content.idx' @('content') { Remove-Item -LiteralPath $idx -Force } 9 -Find -LogPattern $lp
        Scenario 'content index' 'a file named content where the folder belongs' @('content') { [void][TF]::DeleteTree($contentDir); [IO.File]::WriteAllText($contentDir, 'not a folder') } 9 -Find -LogPattern $lp
        Scenario 'content index' 'left-over content.dat.tmp and content.idx.tmp folders' @('content') { File-To-Folder (Join-Path $contentDir 'content.dat.tmp'); File-To-Folder (Join-Path $contentDir 'content.idx.tmp') } 9 -Find -LogPattern $lp
    } 2
    Step 'damaged history, log and crash files' {
        if (-not $S.indexed) { return }
        $h = Join-Path $data 'history.json'
        Scenario 'history' 'history.json cut short' @('history.json') { [IO.File]::WriteAllText($h, '{"recent":["a","b"],"saved":[{"name":"x","que') } 5 -Find
        Scenario 'history' 'history.json of random bytes' @('history.json') { Fill-File $h 65536 'random' } 5 -Find
        Scenario 'history' 'history.json with wrong types' @('history.json') { [IO.File]::WriteAllText($h, '{"recent":5,"saved":"no","opened":{"a":[1,2]},"launches":null,"pinned":[[[[]]]],"version":-1}') } 5 -Find
        Scenario 'history' 'history.json nested 100,000 deep' @('history.json') { [IO.File]::WriteAllText($h, ('[' * 100000)) } 5 -Find
        Scenario 'history' 'history.json of 40 MB' @('history.json') { [IO.File]::WriteAllText($h, ('{"recent":["' + ('x' * 40000000) + '"]}')) } 5 -Find
        Scenario 'history' 'a folder named history.json' @('history.json') { File-To-Folder $h } 5 -Find
        Scenario 'log' 'a folder named log.txt' @('log.txt') { if (Test-Path -LiteralPath $logFile) { Remove-Item -LiteralPath $logFile -Force }; File-To-Folder $logFile } 5 -Find
        Scenario 'log' 'a read-only log.txt of 70 MB' @('log.txt') { Fill-File $logFile 70MB 'zero'; (Get-Item -LiteralPath $logFile).Attributes = 'ReadOnly' } 5 -Find
        # A crash.txt that is not ours: garbage from "last time". The program
        # may ask about it (answered No); it must not choke on it.
        Scenario 'crash file' 'a crash.txt of random bytes from an earlier run' @('crash.txt') {
            [IO.File]::WriteAllBytes($crash, [TD]::RandomBytes(300000, 3))
            $S.plantedHash = (Get-FileHash -LiteralPath $crash -Algorithm SHA256).Hash
            $S.planted = $true
        } 5 -Find
    } 2
    Step 'the data folder itself cannot be written' {
        if (-not $S.indexed) { return }
        [void](Stop-App)
        Restore-Data
        # No program may write, delete or add anything in the folder.
        $r = [TT]::Run('icacls.exe', @($data, '/deny', '*S-1-1-0:(OI)(CI)(W,D,DC)'), 30000)
        if ($r.Code -ne 0) { Note 'state' 'skipped' "icacls could not lock the data folder: $(Short $r.Out 160)"; return }
        $script:aclPaths.Add($data)
        try {
            [void](Start-App)
            $main = Wait-Until { Main-Window } 30
            Start-Sleep -Seconds 8
            [void](Dismiss-Dialogs 'data folder locked')
            $alive = ([TM]::AppCount() -ge 1)
            $ping = if ($main -and $alive) { [TT]::Ping($main.Handle, 15000) } else { -1 }
            Record 'state' 'core' 'starts and stays up when its data folder cannot be written' (($null -ne $main) -and $alive -and $ping -ge 0) "window $(if ($main) { 'yes' } else { 'NO' }), running $alive, answered in $ping ms"
            if ($alive) { [void](Gui-Seed 'data folder cannot be written') }
            [void](Stop-App)
        } finally { Undo-Acls }
        [void](Check-Crash 'data folder cannot be written')
    } 2
    Step 'rebuild after a destroyed index' {
        if (-not $S.indexed) { return }
        Note 'state' 'scenarios run' "$($S.scenarios) damaged-state starts; skipped for time: $($S.skipped)"
        [void](Stop-App)
        Restore-Data
        Fill-File $index 2MB 'random'
        Remove-Item -LiteralPath (Join-Path $S.good 'index.bin') -Force -ErrorAction SilentlyContinue
        [void](Start-App)
        [void](Wait-Main 'with a destroyed index' -Quiet)
        # index.bin exists (garbage), so wait for the log line of the new build.
        $limit = [Math]::Min(20, (Time-Left) - 2)
        $t0 = Get-Date
        $built = Wait-Until { Log-Lines 'index build: \d+ entries' | Select-Object -Last 1 } ($limit * 60) 3000
        $secs = ((Get-Date) - $t0).TotalSeconds
        [void](Health 'rebuilding the index')
        if ($built) {
            Start-Sleep -Seconds 5
            Record 'index' 'core' 'a destroyed index is rebuilt' $true ("{0:N0} s; {1}" -f $secs, $built)
            [void](Check-Seeds 'after rebuilding a destroyed index')
            [void](Gui-Seed 'after rebuilding a destroyed index')
        } else {
            Record 'index' 'probe' 'a destroyed index is rebuilt in time' $false ("no 'index build' line after {0:N0} s; status: {1}" -f $secs, (Short (Status-Text) 160))
        }
    } 6
}

# ================================================================ instances
function Group-Instances {
    Step '20 quick starts' {
        Reset-AppData
        Write-Settings
        $script:logMark = Log-Length
        $procs = @()
        for ($i = 0; $i -lt 20; $i++) { $procs += Start-Process -FilePath $App -PassThru; Start-Sleep -Milliseconds 60 }
        $script:proc = $procs[0]
        # A second copy waits up to 5 s for the first one's window, then leaves.
        $one = Wait-Until { [TM]::AppCount() -eq 1 } 30
        $n = [TM]::AppCount()
        $main = Main-Window
        Record 'instances' 'core' 'after 20 quick starts exactly one copy runs' ($n -eq 1 -and $null -ne $main) "$n processes after 30 s; main window: $(if ($main) { 'yes' } else { 'no' }); log: $(((Log-Lines 'another copy') | Select-Object -Last 2) -join ' | ')"
        [void](Dismiss-Dialogs '20 starts')
        Shot 'twenty-starts'
        [void](Health '20 quick starts')
    }
    Step 'exit during the first indexing' {
        # Still indexing (seconds after the first start). Exit must be prompt
        # and must not leave a half-built index behind as if it were whole.
        Start-Sleep -Seconds 6
        $busy = (Status-Text) -match $IndexingPattern
        $secs = Exit-App 'during the first indexing'
        if (Record-Exit "during the first indexing (indexing shown: $busy)" $secs) {
            Start-Sleep -Seconds 2
            $saved = Test-Path -LiteralPath $index
            $built = (Log-Lines 'index build:') -join ' | '
            # The scan was cut short, so an index.bin written now would be a
            # partial index that the next start takes for the whole one.
            if ($busy) { Record 'exit' 'core' 'an exit during the first indexing does not save a partial index' (-not $saved) "index.bin written: $saved; log: $built" }
            elseif ($saved) { [void](Check-Seeds 'index.bin after an exit right after indexing') }
        } else { [void](Stop-App) }
        if (Test-Path -LiteralPath $index) { Remove-Item -LiteralPath $index -Force -ErrorAction SilentlyContinue }
        [void](Check-Crash 'exit during the first indexing')
    }
    Step 'start and index' {
        [void](Start-App)
        [void](Wait-Main 'after the exit')
        $S.indexed = Wait-Index 20
        [void](Health 'first index')
        if ($S.indexed) { [void](Check-Seeds 'after the first index') }
    } 6
    Step 'a second copy from another folder' {
        # (Also below the scratch Scoop folder, so this copy cannot reach the update server either.)
        $other = Join-Path $scoop 'apps\instafind\other copy\bahth.exe'
        New-Item -ItemType Directory -Force (Split-Path $other) | Out-Null
        Copy-Item -LiteralPath $App -Destination $other -Force
        $first = @(App-Ids)
        $second = Start-Process -FilePath $other -PassThru
        $left = $second.WaitForExit(20000)
        $now = @(App-Ids)
        Record 'instances' 'core' 'a second copy started from another folder steps aside' ($left -and $now.Count -eq 1 -and $now[0] -eq $first[0]) "second copy exited: $left; processes: $($now -join ', ') (first was $($first -join ', '))"
        if (-not $left) { try { $second.Kill() } catch { } }
        # With the command-line flags of a start at sign-in and after an update.
        # --startup must step aside. --updated is how a freshly installed
        # version starts while the old one closes: it may wait for the old
        # copy or take over; either way exactly one copy must be left.
        foreach ($flag in '--startup', '--updated') {
            $before = @(App-Ids)
            $p = Start-Process -FilePath $App -ArgumentList $flag -PassThru
            $left = $p.WaitForExit(80000)
            $one = Wait-Until { [TM]::AppCount() -eq 1 } 15
            $now = @(App-Ids)
            $how = if ($left) { 'it stepped aside' } elseif ($now -contains $p.Id) { 'it took over from the running copy' } else { 'unclear' }
            if ($flag -eq '--startup') { Record 'instances' 'core' "a second copy started with $flag steps aside" ($left -and $now.Count -eq 1) "exited: $left; processes: $($now -join ', ') (before: $($before -join ', '))" }
            else { Record 'instances' 'core' "a copy started with $flag leaves exactly one copy running" ([bool]$one) "$how; processes: $($now -join ', ') (before: $($before -join ', '))" }
            if ($now.Count -gt 1 -and -not $left) { try { $p.Kill() } catch { } }
            if ((Time-Left) -lt 8) { break }
        }
        [void](Wait-Main 'after the second copies' 60 -Quiet)
        [void](Health 'second copies')
    } 3
    Step 'the close button and Alt+F4 hide to the tray' {
        $main = Show-Main
        if (-not $main) { Record 'tray' 'core' 'the window can be shown' $false ''; return }
        [void][TT]::Post($main.Handle, $WM_CLOSE, 0, 0)
        $hidden = Wait-Until { -not (Main-Window -Shown) } 8
        Record 'tray' 'core' 'the close button hides the window and the program keeps running' ([bool]$hidden -and [TM]::AppCount() -eq 1) "processes: $([TM]::AppCount())"
        $back = Show-Main
        Record 'tray' 'core' 'starting the program again shows the window' ($null -ne $back -and [TM]::AppCount() -eq 1) "processes: $([TM]::AppCount())"
        if ($back) {
            Focus-Window $back.Handle
            [TT]::Chord($VK.ALT, $VK.F4)
            $hidden = Wait-Until { -not (Main-Window -Shown) } 8
            Record 'tray' 'probe' 'Alt+F4 (real keystroke) hides the window and the program keeps running' ([bool]$hidden -and [TM]::AppCount() -eq 1) "foreground was ours: $([TT]::GetForegroundWindow() -eq $back.Handle)"
        }
        # Ten times in a row, quickly.
        for ($i = 0; $i -lt 10; $i++) {
            $m = Main-Window
            if ($m) { [void][TT]::Post($m.Handle, $WM_CLOSE, 0, 0) }
            Start-Sleep -Milliseconds 150
            if ($i % 2 -eq 0) { $again = Start-Process -FilePath $App -PassThru; Start-Sleep -Milliseconds 300 }
        }
        Start-Sleep -Seconds 8
        Record 'tray' 'core' 'close and reopen ten times quickly: still exactly one copy' ([TM]::AppCount() -eq 1) "processes: $([TM]::AppCount())"
        [void](Health 'close and reopen')
        if ($S.indexed) { [void](Gui-Seed 'after close and reopen') }
    }
    Step 'signing out (WM_QUERYENDSESSION, WM_ENDSESSION)' {
        $main = Main-Window
        if (-not $main) { return }
        $late = New-Token
        [IO.File]::WriteAllText((Join-Path $Root "seed\$late made before sign-out.txt"), 'x')
        Start-Sleep -Seconds 4
        $q = [TT]::Send($main.Handle, $WM_QUERYENDSESSION, 0, 0, 15000)
        $e = [TT]::Send($main.Handle, $WM_ENDSESSION, 1, 0, 15000)
        Record 'sign-out' 'core' 'the window answers the sign-out messages within 15 s' ($q -ne [long]::MinValue -and $e -ne [long]::MinValue) "WM_QUERYENDSESSION -> $(if ($q -eq [long]::MinValue) { 'no answer' } else { $q }); WM_ENDSESSION -> $(if ($e -eq [long]::MinValue) { 'no answer' } else { $e })"
        Start-Sleep -Seconds 3
        [void](Check-Crash 'sign-out messages')
        # Windows ends the process next.
        [void](Stop-App)
        if ($S.indexed) {
            [void](Check-Seeds 'after signing out')
            $m = Cli-Tokens @($late)
            Record 'sign-out' 'probe' 'a file made seconds before signing out is in the saved index' ($m -and $m.ContainsKey($late)) 'the index is saved about 30 s after a change; sign-out does not wait for that'
        }
        [void](Start-App)
        [void](Wait-Main 'after signing out' -Quiet)
        if ($S.indexed) { [void](Wait-Loaded 60); [void](Gui-Seed 'after signing out') }
        if ($S.indexed) {
            $r = Gui-Find $late $late 8
            Record 'sign-out' 'probe' 'a file made while the program was not running is found after the next start' ($null -ne $r.Hit) 'the program does not walk the drives at start; it follows changes only while it runs'
            Gui-Clear
        }
    } 2
    Step 'Exit from the tray menu' {
        $secs = Exit-App 'idle'
        if (-not (Record-Exit 'idle in the tray' $secs)) { [void](Stop-App) }
        [void](Check-Crash 'exit from the tray')
        if ($S.indexed) { [void](Check-Seeds 'after Exit from the tray') }
        [void](Start-App)
        [void](Wait-Main 'after Exit' -Quiet)
        Record 'instances' 'core' 'after Exit and a new start exactly one copy runs' ([TM]::AppCount() -eq 1) "processes: $([TM]::AppCount())"
    } 2
    Step 'Exit while the index is being saved' {
        if (-not $S.indexed) { return }
        [void](Wait-Loaded 60)
        [void](Wait-Until { Log-Lines '^\d+ watch:' } 20)
        Start-Sleep -Seconds 2
        $late = New-Token
        [IO.File]::WriteAllText((Join-Path $Root "seed\$late made before exit.txt"), 'x')
        $ms = [TM]::WaitForFile($indexTmp, 60000)
        $secs = Exit-App 'during an index save'
        if (Record-Exit "while the index is being saved (index.tmp seen after $ms ms)" $secs) {
            [void](Check-Seeds 'after Exit during an index save')
            $m = Cli-Tokens @($late)
            Record 'exit' 'core' 'a change made before Exit is in the saved index' ($m -and $m.ContainsKey($late)) "[$late]"
        } else { [void](Stop-App) }
        [void](Check-Crash 'exit during an index save')
    } 3
    Step 'the X button with no tray icon and no floating box really exits' {
        [void](Stop-App)
        Write-Settings @{ tray_icon = 0; floating_mode = 'off' }
        [void](Start-App)
        $main = Wait-Main 'without a tray icon' -Quiet
        if (-not $main) { return }
        if ($S.indexed) { [void](Wait-Loaded 60) }
        Start-Sleep -Seconds 3
        $t = Get-Date
        [void][TT]::Post($main.Handle, $WM_CLOSE, 0, 0)
        $gone = Wait-Until { [TM]::AppCount() -eq 0 } 30 100
        $secs = ((Get-Date) - $t).TotalSeconds
        Record 'exit' 'core' 'with no tray icon and no floating box the close button ends the program within 15 s' ([bool]$gone -and $secs -lt 15) ("{0:N1} s" -f $secs)
        if (-not $gone) { [void](Stop-App) }
        [void](Check-Crash 'close button exit')
        if ($S.indexed) { [void](Check-Seeds 'after the close button exit') }
        Write-Settings
    } 2
    Step 'the --moved relaunch' {
        # As after "move to a permanent place": the new copy is started with
        # --moved while the old one is still closing, waits for it, then
        # takes over.
        [void](Start-App)
        [void](Wait-Main 'before the relaunch' -Quiet)
        $old = @(App-Ids)
        $new = Start-Process -FilePath $App -ArgumentList '--moved' -PassThru
        Start-Sleep -Seconds 4
        foreach ($id in $old) { try { Stop-Process -Id $id -Force -ErrorAction Stop } catch { } }
        $took = Wait-Until { $ids = @(App-Ids); if ($ids.Count -eq 1 -and $ids[0] -eq $new.Id -and (Main-Window)) { $true } } 40
        Record 'instances' 'core' 'a copy started with --moved takes over when the old one ends' ([bool]$took) "processes: $((App-Ids) -join ', '); the new one is $($new.Id)"
        [void](Dismiss-Dialogs '--moved')
        [void](Health '--moved relaunch')
        if ($S.indexed) { [void](Wait-Loaded 60); [void](Gui-Seed 'after the --moved relaunch') }
        if ((Time-Left) -gt 4) {
            # And when the old copy does not go away by itself: one of the
            # two must go (the new one gives up, or it takes over).
            $before = @(App-Ids)
            $p = Start-Process -FilePath $App -ArgumentList '--moved' -PassThru
            $left = $p.WaitForExit(90000)
            $one = Wait-Until { [TM]::AppCount() -eq 1 } 15
            $now = @(App-Ids)
            $how = if ($left) { 'it stepped aside' } elseif ($now -contains $p.Id) { 'it took over from the running copy' } else { 'unclear' }
            Record 'instances' 'core' 'a --moved copy started while the first keeps running leaves exactly one copy' ([bool]$one) "$how; processes: $($now -join ', ') (before: $($before -join ', '))"
            if ($now.Count -gt 1 -and -not $left) { try { $p.Kill() } catch { } }
        }
    } 3
}

# ================================================================ content
function Group-Content {
    $S.dir = Join-Path $Root 'content'
    Step 'generate documents' {
        $d = $S.dir
        New-Item -ItemType Directory -Force $d | Out-Null
        $S.docs = New-Object System.Collections.Generic.List[object]    # Name (token in the file name), Word (in the text), What, Kind
        $script:docN = 0x800000
        # A document's file name holds zq<n>, its text the word zc<n>.
        function Doc([string]$What, [string]$Kind = 'probe') {
            $n = $script:docN; $script:docN++
            $o = [pscustomobject]@{ Name = ('zq{0:x6}' -f $n); Word = ('zc{0:x6}' -f $n); What = $What; Kind = $Kind; Path = '' }
            $S.docs.Add($o)
            return $o
        }
        $utf8 = New-Object Text.UTF8Encoding $false
        $v = Join-Path $d 'valid'
        New-Item -ItemType Directory -Force $v | Out-Null
        # Plain text in several encodings.
        $o = Doc 'UTF-8 text' 'core'; [IO.File]::WriteAllText("$v\$($o.Name) utf8.txt", "السلام عليكم $($o.Word) report", $utf8)
        $o = Doc 'UTF-8 text with a BOM'; [IO.File]::WriteAllText("$v\$($o.Name) bom.txt", "مرحبا $($o.Word)", (New-Object Text.UTF8Encoding $true))
        $o = Doc 'UTF-16 LE text'; [IO.File]::WriteAllText("$v\$($o.Name) utf16le.txt", "نص $($o.Word) يونيكود", [Text.Encoding]::Unicode)
        $o = Doc 'UTF-16 BE text'; [IO.File]::WriteAllText("$v\$($o.Name) utf16be.txt", "نص $($o.Word)", [Text.Encoding]::BigEndianUnicode)
        $o = Doc 'Windows-1256 text'; [IO.File]::WriteAllText("$v\$($o.Name) cp1256.txt", "تقرير مالي $($o.Word) سنوي", [Text.Encoding]::GetEncoding(1256))
        $o = Doc 'markdown'; [IO.File]::WriteAllText("$v\$($o.Name).md", "# title`n$($o.Word)", $utf8)
        $o = Doc 'CSV'; [IO.File]::WriteAllText("$v\$($o.Name).csv", "a,b`n1,$($o.Word)", $utf8)
        $o = Doc 'JSON'; [IO.File]::WriteAllText("$v\$($o.Name).json", "{`"k`":`"$($o.Word)`"}", $utf8)
        $o = Doc 'XML'; [IO.File]::WriteAllText("$v\$($o.Name).xml", "<a><b>$($o.Word)</b></a>", $utf8)
        $o = Doc 'HTML'; [IO.File]::WriteAllText("$v\$($o.Name).html", "<html><body><p>hello <b>$($o.Word)</b></p><script>var x=1</script></body></html>", $utf8)
        $o = Doc 'RTF'; [TD]::Rtf("$v\$($o.Name).rtf", "rich $($o.Word) text")
        $o = Doc 'e-mail (.eml)'; [TD]::Eml("$v\$($o.Name).eml", "mail $($o.Word) body")
        $o = Doc 'Word (.docx)'; [TD]::Docx("$v\$($o.Name).docx", "word $($o.Word) document"); $S.docx = "$v\$($o.Name).docx"
        $o = Doc 'PowerPoint (.pptx)'; [TD]::Pptx("$v\$($o.Name).pptx", "slide $($o.Word)"); $S.pptx = "$v\$($o.Name).pptx"
        $o = Doc 'Excel (.xlsx)'; [TD]::Xlsx("$v\$($o.Name).xlsx", "cell $($o.Word)"); $S.xlsx = "$v\$($o.Name).xlsx"
        $o = Doc 'OpenDocument (.odt)'; [TD]::Odt("$v\$($o.Name).odt", "odf $($o.Word)")
        $o = Doc 'PDF'; [TD]::Pdf("$v\$($o.Name).pdf", "pdf $($o.Word) page"); $S.pdf = "$v\$($o.Name).pdf"
        $o = Doc 'text inside a zip'; [TD]::ZipOfText("$v\$($o.Name).zip", 'folder/inside.txt', "zipped $($o.Word)"); $S.zip = "$v\$($o.Name).zip"
        $o = Doc 'text inside a .tar.gz'; [TD]::TarGz("$v\$($o.Name).tar.gz", [string[]]@('dir/inside.txt'), $o.Name, "tarred $($o.Word)"); $S.targz = "$v\$($o.Name).tar.gz"
        $o = Doc 'text inside a .tgz'; [TD]::TarGz("$v\$($o.Name).tgz", [string[]]@('inside.txt'), $o.Name, "tgz $($o.Word)")
        $o = Doc 'text in a .gz'; [TD]::Gz("$v\$($o.Name).txt.gz", [Text.Encoding]::ASCII.GetBytes("gzipped $($o.Word)"))
        $o = Doc 'old Word look-alike (.doc, UTF-16 text)'; [IO.File]::WriteAllBytes("$v\$($o.Name).doc", ([TD]::RandomBytes(600, 1) + [Text.Encoding]::Unicode.GetBytes("old word document $($o.Word) text here") + [TD]::RandomBytes(600, 2)))
        # 7-Zip, when the runner has it: 7z archives and password protection.
        $seven = @((Get-Command 7z.exe -ErrorAction SilentlyContinue | ForEach-Object Source), "$env:ProgramFiles\7-Zip\7z.exe") | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
        $S.seven = $seven
        $plain = Join-Path $Scratch 'plain.txt'
        if ($seven) {
            $o = Doc 'text inside a .7z'; [IO.File]::WriteAllText($plain, "seven $($o.Word)"); $r = [TT]::Run($seven, @('a', '-bd', '-y', "$v\$($o.Name).7z", $plain), 60000); $S.sevenZ = "$v\$($o.Name).7z"
            $pw = Join-Path $d 'password'
            New-Item -ItemType Directory -Force $pw | Out-Null
            [IO.File]::WriteAllText($plain, 'secret zcsecret text')
            $tok = New-Token; [void][TT]::Run($seven, @('a', '-bd', '-y', '-tzip', '-pTORTURE', '-mem=AES256', "$pw\$tok aes.zip", $plain), 60000)
            $tok = New-Token; [void][TT]::Run($seven, @('a', '-bd', '-y', '-tzip', '-pTORTURE', '-mem=ZipCrypto', "$pw\$tok zipcrypto.zip", $plain), 60000)
            $tok = New-Token; [void][TT]::Run($seven, @('a', '-bd', '-y', '-pTORTURE', '-mhe=on', "$pw\$tok encrypted names.7z", $plain), 60000)
            $tok = New-Token; [void][TT]::Run($seven, @('a', '-bd', '-y', '-pTORTURE', "$pw\$tok encrypted.7z", $plain), 60000)
            # Encrypted zips under Office names (a reader meets encrypted members where it expects XML).
            $aes = Get-ChildItem -LiteralPath $pw -Filter '*aes.zip' | Select-Object -First 1
            if ($aes) { foreach ($ext in 'docx', 'xlsx', 'pptx') { $tok = New-Token; Copy-Item -LiteralPath $aes.FullName -Destination "$pw\$tok encrypted zip.$ext" } }
            Note 'content' '7-Zip' "found ($seven): 7z archives and password-protected archives are part of the test"
        } else { Note 'content' '7-Zip' 'not on this runner: no real .7z and no password-protected archives; stubs only' }
        # What an encrypted Office document starts with (an OLE compound file).
        $pw = Join-Path $d 'password'
        New-Item -ItemType Directory -Force $pw | Out-Null
        foreach ($ext in 'docx', 'xlsx', 'pptx', 'doc', 'xls', 'msg') { $tok = New-Token; [TD]::CfbStub("$pw\$tok compound stub.$ext", 20000, 40) }
        # Empty and cut files.
        $bad = Join-Path $d 'damaged'
        New-Item -ItemType Directory -Force $bad | Out-Null
        foreach ($ext in 'pdf', 'docx', 'xlsx', 'xls', 'pptx', 'zip', '7z', 'tar.gz', 'tgz', 'gz', 'tar', 'doc', 'rtf', 'msg', 'eml', 'epub', 'odt', 'ods', 'xlsb', 'png', 'jpg', 'tif', 'html', 'txt') {
            $tok = New-Token; [IO.File]::WriteAllBytes("$bad\$tok empty.$ext", (New-Object byte[] 0))
            $tok = New-Token; [IO.File]::WriteAllBytes("$bad\$tok random.$ext", [TD]::RandomBytes(30000, $script:tokN))
        }
        $whole = @($S.docx, $S.pptx, $S.xlsx, $S.pdf, $S.zip, $S.targz)
        if ($S.sevenZ -and (Test-Path -LiteralPath $S.sevenZ)) { $whole += $S.sevenZ }
        foreach ($src in $whole) {
            $len = (Get-Item -LiteralPath $src).Length
            $ext = if ($src -like '*.tar.gz') { 'tar.gz' } else { [IO.Path]::GetExtension($src).TrimStart('.') }
            foreach ($cut in 1, 4, 22, [long]($len * 0.1), [long]($len * 0.5), [long]($len * 0.9), ($len - 22), ($len - 1)) {
                if ($cut -le 0 -or $cut -ge $len) { continue }
                $tok = New-Token; [TD]::Truncated($src, "$bad\$tok cut at $cut.$ext", $cut)
            }
            # One byte changed, at a few places.
            foreach ($at in 0, 10, [long]($len / 2), ($len - 5)) {
                $tok = New-Token; $p = "$bad\$tok byte $at changed.$ext"; Copy-Item -LiteralPath $src -Destination $p; Patch-File $p $at ([byte[]](0xFF))
            }
        }
        # Wrong extensions.
        $wx = Join-Path $d 'wrong extension'
        New-Item -ItemType Directory -Force $wx | Out-Null
        foreach ($pair in @(@($S.zip, 'pdf'), @($S.pdf, 'docx'), @($S.docx, 'xls'), @($S.xlsx, 'pptx'), @($S.pptx, 'xlsx'), @($S.pdf, 'zip'), @($S.targz, 'zip'), @($S.zip, 'tar.gz'), @($S.docx, 'txt'), @($S.pdf, 'jpg'), @($S.zip, 'epub'), @($S.docx, 'odt'), @($S.xlsx, 'ods'), @($S.pdf, 'eml'), @($S.zip, 'msg'), @($S.docx, 'rtf'))) {
            $tok = New-Token; Copy-Item -LiteralPath $pair[0] -Destination "$wx\$tok really $([IO.Path]::GetExtension($pair[0]).TrimStart('.')).$($pair[1])"
        }
        # Binary files named as text; the start of a program named .txt.
        $bin = Join-Path $d 'binary'
        New-Item -ItemType Directory -Force $bin | Out-Null
        $tok = New-Token; [IO.File]::WriteAllBytes("$bin\$tok random 5mb.txt", [TD]::RandomBytes(5MB, 21))
        $tok = New-Token; [IO.File]::WriteAllBytes("$bin\$tok zeros 5mb.txt", (New-Object byte[] (5MB)))
        $tok = New-Token; $image = [IO.File]::ReadAllBytes($App); $head = New-Object byte[] ([Math]::Min($image.Length, 2MB)); [Array]::Copy($image, $head, $head.Length); [IO.File]::WriteAllBytes("$bin\$tok program start.txt", $head); $image = $null
        $tok = New-Token; [IO.File]::WriteAllBytes("$bin\$tok bad utf8.txt", ([byte[]](0xC3, 0x28, 0xA0, 0xA1, 0xE2, 0x28, 0xA1, 0xF0, 0x90, 0x28, 0xBC, 0xFF, 0xFE, 0xFD, 0xED, 0xA0, 0x80) * 4000))
        $tok = New-Token; [IO.File]::WriteAllBytes("$bin\$tok half utf16.txt", ([byte[]](0xFF, 0xFE) + [Text.Encoding]::Unicode.GetBytes('odd length text') + [byte[]](0x41)))
        $tok = New-Token; [IO.File]::WriteAllText("$bin\$tok one word 3mb.txt", ('w' * 3000000), $utf8)
        $tok = New-Token; [IO.File]::WriteAllText("$bin\$tok only newlines.txt", ("`n" * 2000000), $utf8)
        $tok = New-Token; [IO.File]::WriteAllText("$bin\$tok only tatweel.txt", ('ـ' * 1000000), $utf8)
        $tok = New-Token; [IO.File]::WriteAllText("$bin\$tok nested html.html", (('<div>' * 200000) + 'deep' + ('</div>' * 200000)), $utf8)
        $tok = New-Token; [IO.File]::WriteAllText("$bin\$tok entities.html", ('&amp;&#x202E;&#99999999999;&bogus;&#xD800;&' * 100000), $utf8)
        $tok = New-Token; [IO.File]::WriteAllText("$bin\$tok unclosed rtf.rtf", ('{\rtf1' + ('{\b ' * 100000) + '\u-1 \u99999999 \bin999999999 x'), $utf8)
        $tok = New-Token; [IO.File]::WriteAllText("$bin\$tok deep json.json", ('[' * 500000), $utf8)
        # Big files: the program reads at most 20 MB of a text file and skips files over 100 MB.
        $big = Join-Path $d 'big'
        New-Item -ItemType Directory -Force $big | Out-Null
        if (Have-Space 1 'the big text files') {
            $o = Doc 'the first line of a 99 MB text file'; $S.big99end = "zc99end"; [TD]::BigText("$big\$($o.Name) 99mb.txt", 99MB, 100, $o.Word, 'zc99end')
            $o = Doc 'the first line of a 101 MB text file (over the limit)' 'note'; [TD]::BigText("$big\$($o.Name) 101mb.txt", 101MB, 100, $o.Word, 'zc101end')
            $o = Doc 'the start of a 20 MB single line'; [TD]::BigText("$big\$($o.Name) 20mb one line.txt", 20MB, 0, $o.Word, 'zc20end')
            $o = Doc 'the start of a 50 MB log of short lines'; [TD]::BigText("$big\$($o.Name) 50mb.log", 50MB, 12, $o.Word, 'zc50end')
        }
        # Bombs: small on disk, huge when unpacked. Only the program under test ever opens them.
        $bm = Join-Path $d 'bombs'
        New-Item -ItemType Directory -Force $bm | Out-Null
        $tok = New-Token; [TD]::ZeroZip("$bm\$tok 512mb of zeros.zip", 'zeros.txt', 512MB)
        $tok = New-Token; [TD]::ZeroZip("$bm\$tok zeros as document xml.docx", 'word/document.xml', 512MB)
        $tok = New-Token; [TD]::ZeroZip("$bm\$tok zeros as slide.pptx", 'ppt/slides/slide1.xml', 256MB)
        $tok = New-Token; [TD]::ZeroZip("$bm\$tok zeros as content xml.odt", 'content.xml', 256MB)
        $tok = New-Token; [TD]::XlsxBomb("$bm\$tok zeros in the workbook.xlsx", 512MB)
        $tok = New-Token; [TD]::XlsxCorners("$bm\$tok cells at both corners.xlsx", 'corner')
        $tok = New-Token; [TD]::ZeroGz("$bm\$tok 512mb of zeros.gz", 512MB)
        $tok = New-Token; [TD]::ZeroGz("$bm\$tok 512mb of zeros.txt.gz", 512MB)
        $tok = New-Token; [TD]::ZeroTarGz("$bm\$tok 512mb member.tar.gz", 512MB)
        $tok = New-Token; [TD]::NestedZip("$bm\$tok zip in zip 8 deep.zip", 8, 256MB, 'bottom zcnested')
        $tok = New-Token; [TD]::NestedZip("$bm\$tok zip in zip 40 deep.zip", 40, 1MB, 'bottom zcnested40')
        $tok = New-Token; [TD]::ManyMembersZip("$bm\$tok 5000 members.zip", 5000, 'member')
        $tok = New-Token; [TD]::DeepNameZip("$bm\$tok member 300 folders deep.zip", 300, 'deep member')
        $tok = New-Token; [IO.File]::WriteAllText("$bm\$tok entity bomb.xml", [TD]::XmlBomb($tok), $utf8)
        $tok = New-Token; [IO.File]::WriteAllText("$bm\$tok entity bomb.html", [TD]::XmlBomb($tok), $utf8)
        $tok = New-Token; [TD]::DocxXmlBomb("$bm\$tok entity bomb.docx", $tok)
        # Member names that try to leave the folder.
        $S.trav = Join-Path $d 'traversal\a\b'
        New-Item -ItemType Directory -Force $S.trav | Out-Null
        $S.travTok = New-Token
        [TD]::TraversalZip("$($S.trav)\$($S.travTok) names.zip", $S.travTok, 'traversal zctraversal')
        [TD]::TarGz("$($S.trav)\$($S.travTok) names.tar.gz", [TD]::TraversalNames, $S.travTok, 'traversal zctraversal')
        Copy-Item -LiteralPath "$($S.trav)\$($S.travTok) names.zip" -Destination "$($S.trav)\$($S.travTok) names.docx"
        Copy-Item -LiteralPath "$($S.trav)\$($S.travTok) names.zip" -Destination "$($S.trav)\$($S.travTok) names.epub"
        # 5,000 small documents, each with one unique word.
        $S.smallBase = 0x900000; $S.smallCount = 5000
        $made = [TD]::SmallDocs((Join-Path $d 'small'), $S.smallBase, $S.smallCount)
        $files = @(Get-ChildItem -LiteralPath $d -Recurse -File -ErrorAction SilentlyContinue)
        Note 'content' 'generated' ("{0:N0} files, {1:N0} MB on disk ({2} small documents, {3} valid documents with a word to find); free space now {4} GB" -f $files.Count, (($files | Measure-Object Length -Sum).Sum / 1MB), $made, $S.docs.Count, (Free-GB))
    }
    Step 'start and index' { [void](Fresh-Start) } 8

    # One search inside documents for any of `Words` among the files changed
    # in the last day. Returns the lower-case names of the rows; $null when
    # the search did not finish in `Wait` seconds. (Callers take it with
    # @(...): a search that finds nothing gives an empty list, not $null,
    # so "finished" is kept in $S.searchDone.)
    function Content-Search([string[]]$Words, [int]$Wait = 300) {
        $S.searchDone = $false
        $u = Ui
        if (-not $u -or -not $u.Content -or -not $u.Search) { $S.lastSearch = 'no checkbox or Search button'; return }
        if ([TT]::Send($u.Content.Handle, $BM_GETCHECK, 0, 0, 5000) -ne 1) {
            [void][TT]::Post($u.Content.Handle, $BM_CLICK, 0, 0)
            [void](Wait-Until { [TT]::Send($u.Content.Handle, $BM_GETCHECK, 0, 0, 5000) -eq 1 } 5)
        }
        $before = @(Log-Lines 'content search:').Count
        [void][TT]::SetText($u.Query.Handle, ('dm:1d ' + ($Words -join '|')), 10000)
        Start-Sleep -Milliseconds 300
        [void][TT]::Post($u.Search.Handle, $BM_CLICK, 0, 0)
        $t = Get-Date
        $done = $null
        while (((Get-Date) - $t).TotalSeconds -lt $Wait -and -not [TM]::Expired) {
            Start-Sleep -Seconds 2
            $lines = @(Log-Lines 'content search:')
            if ($lines.Count -gt $before) { $done = $lines[-1]; break }
            if ([TM]::AppCount() -eq 0) { break }
            [void](Check-Responsive 'search inside documents')
        }
        $S.lastSearch = "{0:N0} s; {1}; status: {2}" -f ((Get-Date) - $t).TotalSeconds, $done, (Short (Status-Text) 200)
        Detail "content search for $($Words.Count) words: $($S.lastSearch)"
        if (-not $done) { return }
        $S.searchDone = $true
        Start-Sleep -Seconds 1
        return @(List-Names 400 | ForEach-Object { $_.ToLowerInvariant() })
    }
    Step 'search inside documents: the small documents' {
        if (-not $S.indexed) { Note 'content' 'skipped' 'no index'; return }
        [void](Show-Main)
        $u = Ui
        Record 'content' 'core' 'the "inside documents" box and the Search button are there' ($u -and $u.Content -and $u.Search) ''
        $rng = New-Object Random 13
        $S.pick = @(1..15 | ForEach-Object { $S.smallBase + $rng.Next($S.smallCount) } | Select-Object -Unique)
        $rows = @(Content-Search @($S.pick | ForEach-Object { 'zc{0:x6}' -f $_ }))
        Shot 'content-small'
        [void](Health 'search inside documents (first)')
        Record 'content' 'core' 'a search inside documents over the torture files finishes within 5 minutes' ([bool]$S.searchDone) $S.lastSearch
        if (-not $S.searchDone) { return }
        $missing = @($S.pick | Where-Object { $name = 'zq{0:x6}' -f $_; -not ($rows | Where-Object { $_.Contains($name) }) })
        Record 'content' 'core' 'unique words in small text documents are found (sample of the 5,000)' ($missing.Count -eq 0) "$($S.pick.Count) sampled, $($rows.Count) rows; missing: $(($missing | ForEach-Object { 'zq{0:x6}' -f $_ }) -join ' ')"
    } 3
    Step 'search inside documents: every format' {
        if (-not $S.indexed) { return }
        $rows = @(Content-Search @($S.docs | ForEach-Object Word))
        Shot 'content-formats'
        [void](Health 'search inside documents (formats)')
        if (-not $S.searchDone) { Record 'content' 'core' 'the search over every document format finishes within 5 minutes' $false $S.lastSearch; return }
        foreach ($o in $S.docs) {
            $found = [bool]($rows | Where-Object { $_.Contains($o.Name) })
            if ($o.Kind -eq 'note') { Note 'content' $o.What $(if ($found) { 'found' } else { 'not found' }) }
            else { Record 'content' $o.Kind "a word is found in: $($o.What)" $found $(if (-not $found) { $S.lastSearch } else { '' }) }
        }
        # Words past the first 20 MB of a text file, and inside traps.
        $rows = @(Content-Search @('zc99end', 'zc101end', 'zc20end', 'zc50end', 'zcnested', 'zcnested40', 'zctraversal', 'zcsecret'))
        if ($S.searchDone) { Note 'content' 'words at the end of big files, in nested zips, in archives with odd member names, in protected archives' "rows: $(($rows | Select-Object -First 12 | ForEach-Object { Short $_ 40 }) -join ' | ')" }
        [void](Health 'search inside documents (ends and traps)')
        # Nothing an archive names may appear on disk.
        $escaped = @(Get-ChildItem -LiteralPath (Join-Path $S.dir 'traversal\a'), (Join-Path $S.dir 'traversal'), $S.dir, $Root, "$env:SystemDrive\" -Filter "*escaped-$($S.travTok)*" -File -ErrorAction SilentlyContinue)
        Record 'content' 'core' 'no member of an archive with ../ names was written to disk' ($escaped.Count -eq 0) (($escaped | ForEach-Object FullName) -join ', ')
    } 3
    Step 'background content indexing' {
        if (-not $S.indexed) { return }
        # The drives of a runner hold hundreds of thousands of documents, so
        # this may not finish in the time there is: a probe.
        $limit = [Math]::Min(12, (Time-Left) - 6)
        $t0 = Get-Date; $warm = $null
        while (((Get-Date) - $t0).TotalMinutes -lt $limit -and -not [TM]::Expired) {
            $warm = Log-Lines 'content index warm:' | Where-Object { $_ -notmatch 'stopped' } | Select-Object -Last 1
            if ($warm) { break }
            if (-not (Health 'while documents are indexed in the background')) { break }
            Start-Sleep -Seconds 20
        }
        $size = if (Test-Path -LiteralPath $contentDir) { (Get-ChildItem -LiteralPath $contentDir -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1MB } else { 0 }
        Record 'content' 'probe' 'background content indexing of all drives finishes in the time there is' ($null -ne $warm) ("waited {0:N1} min; content store {1:N0} MB; {2}; status: {3}" -f ((Get-Date) - $t0).TotalMinutes, $size, $warm, (Short (Status-Text) 160))
        Note 'content' 'content index' (((Log-Lines 'content index') | Select-Object -Last 3) -join ' | ')
        # The same sample again, now (partly) from the content index.
        $rows = @(Content-Search @($S.pick | ForEach-Object { 'zc{0:x6}' -f $_ }))
        if (-not $S.searchDone) { Record 'content' 'core' 'the second search inside documents finishes within 5 minutes' $false $S.lastSearch }
        else {
            $missing = @($S.pick | Where-Object { $name = 'zq{0:x6}' -f $_; -not ($rows | Where-Object { $_.Contains($name) }) })
            Record 'content' 'core' 'the small documents are found again after background indexing ran' ($missing.Count -eq 0) "missing $($missing.Count); $($S.lastSearch)"
        }
        [void](Health 'after background content indexing')
    } 4
    Step 'restart with the content index' {
        if (-not $S.indexed) { return }
        Keep-Log 'content'
        $secs = Exit-App 'after the document searches'
        if (-not (Record-Exit 'after the document searches' $secs)) { [void](Stop-App) }
        [void](Check-Crash 'exit after the document searches')
        [void](Start-App)
        [void](Wait-Main 'after the document searches' -Quiet)
        [void](Wait-Loaded 60)
        Start-Sleep -Seconds 10
        $open = (Log-Lines 'content index open') | Select-Object -Last 1
        Record 'content' 'core' 'the content index opens after a restart' ($open -and $open -notmatch 'open failed') "$open"
        $rows = @(Content-Search @($S.pick | Select-Object -First 5 | ForEach-Object { 'zc{0:x6}' -f $_ }))
        Record 'content' 'core' 'search inside documents works after a restart' ($S.searchDone -and $rows.Count -ge 1) $S.lastSearch
        [void](Health 'after the restart')
    } 3
}

# ================================================================ soak
function Group-Soak {
    $S.dir = Join-Path $Root 'soak'
    $S.base = 0xA00000
    Step 'start in the tray and index' {
        New-Item -ItemType Directory -Force $S.dir | Out-Null
        [void](Fresh-Start @('--startup') 14)
    } 8
    Step 'soak' {
        # (Eight minutes are kept for the checks after it.)
        $span = [Math]::Min($SoakMinutes, (Time-Left) - 8)
        if ($span -lt 5) { Note 'soak' 'skipped' ("only {0:N1} min left" -f (Time-Left)); return }
        if ($span -lt $SoakMinutes - 0.5) { Note 'soak' 'shortened' ("{0:N1} minutes instead of {1}: the first index took long" -f $span, $SoakMinutes) }
        # 1.0's idle trim lines, gathered as they come (log.txt is emptied
        # past 64 KB, which two hours can reach).
        $trimSeen = New-Object 'System.Collections.Generic.List[string]'
        Start-Sleep -Seconds 20          # let the start settle before the slope is measured
        [TM]::Phase = 'soak'
        [TC]::StartSoak($S.dir, $S.base, 8, 17)
        $t0 = Get-Date; $searches = 0; $badCli = @(); $slowest = 0; $round = 0
        $queries = @('zq', 'تقرير', 'report ext:txt', 'ext:dll size:>1mb', 'zqa0 dm:1d', '*.txt', 'folder:seed', 'regex:^zq.*c\.txt$', 'fuzzy:reprot', 'budget|ميزانية', 'ـــ')
        while (((Get-Date) - $t0).TotalMinutes -lt $span -and -not [TM]::Expired) {
            Start-Sleep -Seconds 20
            $round++
            if ($S.indexed) {
                $q = $queries[$round % $queries.Count]
                $r = Run-Cli @('-s', $q, '--count', '--all') 120
                $searches++
                if ($r.Ms -gt $slowest) { $slowest = $r.Ms }
                if ($r.Code -lt 0 -or $r.Code -gt 2) { $badCli += "'$q' exit $($r.Code)" }
            }
            # The hidden window searches too, as a person coming back to it.
            if ($round % 3 -eq 0) { [void](Gui-Find ('zqa0' + ('{0:x2}' -f ($round % 256))) '' 0); Gui-Clear }
            if (-not (Health "soak minute $([int]((Get-Date) - $t0).TotalMinutes)")) { break }
            if ($round % 15 -eq 0) {
                foreach ($l in @(Log-Lines 'memory: (idle trim|private|passed)')) { if (-not $trimSeen.Contains([string]$l)) { $trimSeen.Add([string]$l) } }
                Say ("  soak {0:N0} min: {1} file operations, {2} searches, {3} files alive" -f ((Get-Date) - $t0).TotalMinutes, [TC]::SoakOps, $searches, [TC]::SoakLive())
            }
        }
        [TC]::StopSoak()
        $S.soakMinutes = ((Get-Date) - $t0).TotalMinutes
        [TM]::Phase = 'after soak'
        Note 'soak' 'ran' ("{0:N1} minutes: {1:N0} file operations ({2} failed), {3} command-line searches (slowest {4} ms)" -f $S.soakMinutes, [TC]::SoakOps, [TC]::SoakErrors, $searches, $slowest)
        Record 'soak' 'core' 'every command-line search during the soak ended with exit 0, 1 or 2' ($badCli.Count -eq 0) ($badCli -join '; ')
        Record 'soak' 'core' 'the program is running and answering after the soak' (Health 'after the soak') ''
        # The soak types nothing in the program's windows (its searches are
        # the command line's and messages to the hidden window), so 1.0's
        # idle trim may run in it: its lines and the memory guard's.
        foreach ($l in @(Log-Lines 'memory: (idle trim|private|passed)')) { if (-not $trimSeen.Contains([string]$l)) { $trimSeen.Add([string]$l) } }
        $trims = @($trimSeen | Where-Object { $_ -match 'memory: idle trim:' })
        $mem = @($trimSeen | Where-Object { $_ -match 'memory: (private|passed)' })
        Note 'soak' 'idle trims and memory lines in log.txt' ("{0} idle trims: {1}; memory lines: {2}" -f $trims.Count, ((@($trims | Select-Object -Last 4) | ForEach-Object { ($_ -replace '^\d+ ', '') }) -join ' | '), ((@($mem | Select-Object -Last 3) | ForEach-Object { ($_ -replace '^\d+ ', '') }) -join ' | '))
        Keep-Log 'soak'
    } 6
    Step 'the index matches the disk after the soak' {
        if (-not $S.indexed) { return }
        # The index is saved about 30 s after the last change.
        Start-Sleep -Seconds ([int][Math]::Max(0, [Math]::Min(40, ((Time-Left) - 1.5) * 60)))
        $expect = New-Object System.Collections.Generic.List[object]
        foreach ($i in [TC]::SoakSample($true, 100, 3)) { $expect.Add(@{ Token = ('zq{0:x6}' -f ($S.base + $i)); Present = $true; Under = '\soak\'; What = 'made in the soak' }) }
        foreach ($i in [TC]::SoakSample($false, 100, 4)) { $expect.Add(@{ Token = ('zq{0:x6}' -f ($S.base + $i)); Present = $false; Under = ''; What = 'deleted or renamed in the soak' }) }
        [void](Wait-Converged $expect 12 'soak' 'after the soak the saved index matches the disk (random sample)')
    } 2
}

# ================================================================ run
function Memory-Checks {
    $csv = Join-Path $Out 'memory.csv'
    $rows = @()
    try { $rows = @([TM]::Analyze($csv)) } catch { Note 'memory' 'analysis' "failed: $_"; return @() }
    $flags = @()
    foreach ($r in $rows) {
        if ($r.Samples -lt 6) { continue }
        if ($r.SettledMB -gt 1536) { $flags += ("{0}: settled at {1:N0} MB private" -f $r.Phase, $r.SettledMB) }
        if ($r.Minutes -ge 8 -and $r.SlopeMBperMin -gt 3 -and ($r.LastMB - $r.FirstMB) -gt 100) { $flags += ("{0}: private bytes grew {1:N1} MB a minute for {2:N0} min ({3:N0} to {4:N0} MB)" -f $r.Phase, $r.SlopeMBperMin, $r.Minutes, $r.FirstMB, $r.LastMB) }
        if (($r.HandlesLast - $r.HandlesFirst) -gt 1000) { $flags += ("{0}: handles {1} to {2}" -f $r.Phase, $r.HandlesFirst, $r.HandlesLast) }
        if (($r.ThreadsLast - $r.ThreadsFirst) -gt 40) { $flags += ("{0}: threads {1} to {2}" -f $r.Phase, $r.ThreadsFirst, $r.ThreadsLast) }
    }
    $peak = ($rows | Measure-Object PeakMB -Maximum).Maximum
    Record 'memory' 'probe' 'private bytes settle under 1.5 GB, no steady growth, no handle or thread build-up' ($flags.Count -eq 0) ("peak {0:N0} MB private; {1}" -f $peak, ($flags -join '; '))
    if ($Group -eq 'soak') {
        $soak = $rows | Where-Object { $_.Phase -eq 'soak' } | Sort-Object Samples -Descending | Select-Object -First 1
        if ($soak) {
            $text = "{0:N2} MB a minute over {1:N0} min ({2:N0} to {3:N0} MB private, peak {4:N0}); threads {5} to {6} (max {7}); handles {8} to {9} (max {10})" -f $soak.SlopeMBperMin, $soak.Minutes, $soak.FirstMB, $soak.LastMB, $soak.PeakMB, $soak.ThreadsFirst, $soak.ThreadsLast, $soak.ThreadsMax, $soak.HandlesFirst, $soak.HandlesLast, $soak.HandlesMax
            Note 'soak' 'memory slope, threads and handles' $text
            Record 'soak' 'probe' 'memory slope during the soak under 1 MB a minute (or under 30 MB in all)' ($soak.SlopeMBperMin -lt 1 -or ($soak.LastMB - $soak.FirstMB) -lt 30) $text
            Record 'soak' 'probe' 'no thread or handle growth during the soak' (($soak.ThreadsLast - $soak.ThreadsFirst) -le 10 -and ($soak.HandlesLast - $soak.HandlesFirst) -le 200) $text
        }
    }
    return $rows
}
function Write-Report($Memory) {
    $fail = @($results | Where-Object Result -eq 'FAIL').Count
    $miss = @($results | Where-Object Result -eq 'MISS').Count
    $pass = @($results | Where-Object Result -eq 'PASS').Count
    $md = New-Object Text.StringBuilder
    [void]$md.AppendLine("## Torture test, group ``$Group``: $pass passed, $fail failed, $miss findings")
    [void]$md.AppendLine('')
    [void]$md.AppendLine(("Ran {0:N1} of {1} minutes. FAIL = a core check (crash report, program gone, window not answering, unreadable index, a file that exists and is not found). MISS = a finding that does not fail the job. Details, screenshots, ``memory.csv`` and ``run.log`` are in the ``torture-$Group`` artifact." -f ((Get-Date) - $started).TotalMinutes, $Minutes))
    [void]$md.AppendLine('')
    [void]$md.AppendLine('| Result | Kind | Area | Check | Detail |')
    [void]$md.AppendLine('|---|---|---|---|---|')
    # Failures and findings first.
    $order = @{ FAIL = 0; MISS = 1; PASS = 2; NOTE = 3 }
    $n = 0
    foreach ($r in ($results | Sort-Object { $order[$_.Result] }, Seq)) {
        $d = Short (($r.Detail -replace '\|', '/') -replace '[<>]', ' ') 400
        $c = ($r.Check -replace '\|', '/') -replace '[<>]', ' '
        [void]$md.AppendLine("| $($r.Result) | $($r.Kind) | $($r.Area) | $c | $d |")
        $n++
        if ($n -ge 400) { [void]$md.AppendLine("| NOTE | note | report | cut after 400 rows | the rest is in run.log |"); break }
    }
    if ($Memory -and @($Memory).Count) {
        [void]$md.AppendLine('')
        [void]$md.AppendLine('### Memory (sampled every 5 s; one row for each step and process)')
        [void]$md.AppendLine('')
        [void]$md.AppendLine('| Step | PID | Minutes | Private MB first / last / peak | Settled MB | MB per min | Working set peak | Threads first / last / max | Handles first / last / max |')
        [void]$md.AppendLine('|---|---|---|---|---|---|---|---|---|')
        foreach ($m in @($Memory | Where-Object { $_.Samples -ge 3 } | Select-Object -First 60)) {
            [void]$md.AppendLine(("| {0} | {1} | {2:N1} | {3:N0} / {4:N0} / {5:N0} | {6:N0} | {7:N2} | {8:N0} | {9} / {10} / {11} | {12} / {13} / {14} |" -f ($m.Phase -replace '\|', '/'), $m.Pid, $m.Minutes, $m.FirstMB, $m.LastMB, $m.PeakMB, $m.SettledMB, $m.SlopeMBperMin, $m.WsPeakMB, $m.ThreadsFirst, $m.ThreadsLast, $m.ThreadsMax, $m.HandlesFirst, $m.HandlesLast, $m.HandlesMax))
        }
    }
    [IO.File]::WriteAllText((Join-Path $Out 'report.md'), $md.ToString(), (New-Object Text.UTF8Encoding $false))
    return $fail
}
function Teardown {
    Say '--- clean-up'
    [TM]::Phase = 'clean-up'
    try { [TC]::StopSoak() } catch { }
    [TM]::Stop = $true
    $t = Get-Date
    while ([TJ]::Busy -and ((Get-Date) - $t).TotalSeconds -lt 20) { Start-Sleep -Milliseconds 200 }
    try { Keep-Log 'end' } catch { }
    try { if (Test-Path -LiteralPath $settingsFile) { Copy-Item -LiteralPath $settingsFile -Destination (Join-Path $Out 'settings-end.txt') -Force -ErrorAction SilentlyContinue } } catch { }
    try { [void](Check-Crash 'end of the run') } catch { }
    try { [void](Stop-App) } catch { }
    try { Release-Locks } catch { }
    try { Undo-Acls } catch { }
    try { Remove-Subst } catch { }
    try { if ($script:vhd) { [void](Detach-Vhd) } } catch { }
    # Everything the test made goes away again (the runner is thrown away
    # after the job, but its disk is not ours to fill).
    foreach ($p in @($Root, $Scratch, $data)) {
        try {
            if ([TF]::Exists($p)) { $left = [TF]::DeleteTree($p); if ($left) { Detail "clean-up: $left entries of $p could not be deleted (error $([TF]::LastError))" } }
        } catch { Detail "clean-up of $p failed: $_" }
    }
    foreach ($d in [IO.DriveInfo]::GetDrives()) {
        if ($d.DriveType -ne 'Fixed' -or -not $d.IsReady) { continue }
        foreach ($n in 'ifx-other', 'ifx-mv') { $p = Join-Path $d.Name $n; try { if ([TF]::Exists($p)) { [void][TF]::DeleteTree($p) } } catch { } }
    }
    try {
        Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name BahthSari -ErrorAction SilentlyContinue
        Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' -Name BahthSari -ErrorAction SilentlyContinue
        Remove-Item 'HKCU:\Software\BahthSari' -Recurse -Force -ErrorAction SilentlyContinue
    } catch { }
    Detail ("clean-up took {0:N0} s; free space {1} GB" -f ((Get-Date) - $t).TotalSeconds, (Free-GB))
}

$fail = 1
try {
    Say ("torture test '{0}': PowerShell {1}, {2}, screen {3}x{4}, {5} GB free on {6}, budget {7} min" -f $Group, $PSVersionTable.PSVersion, [Environment]::OSVersion.VersionString, [Windows.Forms.SystemInformation]::VirtualScreen.Width, [Windows.Forms.SystemInformation]::VirtualScreen.Height, (Free-GB), $env:SystemDrive, $Minutes)
    # The watchdog: past the budget it kills bahth and the test's own
    # children, and every loop here stops, so the report is still written.
    [TM]::StartWatchdog([int](($Minutes - 1) * 60))
    [TM]::StartSampler((Join-Path $Out 'memory.csv'), 5000)
    try {
        [void][TM]::KillApp()
        foreach ($p in @($Root, $Scratch)) { if ([TF]::Exists($p)) { [void][TF]::DeleteTree($p) } }
        New-Item -ItemType Directory -Force $Root, $Scratch, (Split-Path $App) | Out-Null
        Copy-Item -LiteralPath $Exe -Destination $App -Force
        $env:SCOOP = $scoop
        # The classic window for every start, whatever settings.txt says
        # (1.0: the new look is the only one for users); the copies the
        # program starts itself inherit it. Nothing forces the new look.
        $env:BAHTH_CLASSIC = '1'
        foreach ($v in 'BAHTH_WEBUI', 'BAHTH_WEBUI_FAIL') { Remove-Item "Env:$v" -ErrorAction SilentlyContinue }
        $r = [TT]::Run($App, @('--version'), 30000)
        Note 'setup' 'program' "$(Short $r.Out 60) ($((Get-Item -LiteralPath $App).Length) bytes) at $App"
        if ($r.Code -ne 0) { Record 'setup' 'core' 'the program answers --version' $false "exit $($r.Code) $(Short $r.Err 200)" }
        New-Seeds
        switch ($Group) {
            'names' { Group-Names }
            'tree' { Group-Tree }
            'churn' { Group-Churn }
            'state' { Group-State }
            'instances' { Group-Instances }
            'content' { Group-Content }
            'soak' { Group-Soak }
        }
        if ([TM]::Expired) { Record 'time' 'probe' 'the group finished inside its time budget' $false "the watchdog ended it after $($Minutes - 1) minutes; later steps did not run" }
        Scan-Update
        Record 'setup' 'core' 'the copy under test never contacted the update server (no network use)' ($script:updateLines.Count -eq 0) (($script:updateLines | Select-Object -First 3) -join ' | ')
        Record 'stability' 'core' 'no crash report in the whole run' ($script:crashes -eq 0) "$($script:crashes) crash reports (copies in the artifact)"
        Record 'stability' 'probe' 'the window never took longer than 3 s to answer' ($script:pingSlow -eq 0) ("{0} checks, slowest {1} ms, {2} over 3 s" -f $script:pings, $script:pingMax, $script:pingSlow)
    } catch {
        Record 'harness' 'core' 'the test ran to the end' $false ("{0} (line {1})" -f $_.Exception.Message, $_.InvocationInfo.ScriptLineNumber)
        Detail $_.ScriptStackTrace
    } finally {
        Shot 'end'
        try { Teardown } catch { Detail "clean-up failed: $_" }
        [TM]::StopSampler()
        [TM]::StopWatchdog()
        $memory = @()
        try { $memory = @(Memory-Checks) } catch { Detail "memory checks failed: $_" }
        $fail = Write-Report $memory
    }
} finally {
    $runLog.Dispose()
}
Write-Host ("torture test '{0}' done in {1:N1} min: {2} core failures" -f $Group, ((Get-Date) - $started).TotalMinutes, $fail)
if ($fail -gt 0) { exit 1 }
