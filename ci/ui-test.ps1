# Automated "real user" test of InstaFind on a real Windows desktop.
#
# Drives the exact published exe the way a person does: real keystrokes
# (SendInput), real clicks through UI Automation, the global hotkey, the close
# button. It covers the first-run question, the floating box, Arabic and
# English search in the main window, search inside documents, the right-click
# and Options menus, the quick search popup (apps, answers, files), the
# Arabic interface and close-to-tray. Every step saves a screenshot.
#
# Two kinds of checks:
#   core  - must work; a failure fails the job.
#   probe - a feature we expect to work (typo, wrong keyboard layout, Arabizi,
#           Arabic app names...). A miss is reported as a finding, not a
#           failure, so it can never block a release by itself.
#
# Output: <Out>\report.md (also added to the job summary) and <Out>\*.png.
# Runs under Windows PowerShell 5.1 (it ships UI Automation for .NET).
# This file is UTF-8 with a BOM so Windows PowerShell reads the Arabic in it.
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Out = (Join-Path $env:RUNNER_TEMP 'ui-test')
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class U {
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }
    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    public struct RECT { public int Left, Top, Right, Bottom; }

    const uint KEYUP = 2, UNICODE = 4;

    static INPUT Key(ushort vk, ushort scan, uint flags) {
        var i = new INPUT { type = 1 };
        i.u.ki = new KEYBDINPUT { wVk = vk, wScan = scan, dwFlags = flags };
        return i;
    }

    // Types text as a person would, one character at a time.
    public static void Type(string text) {
        foreach (char c in text) {
            var keys = new[] { Key(0, c, UNICODE), Key(0, c, UNICODE | KEYUP) };
            SendInput(2, keys, Marshal.SizeOf(typeof(INPUT)));
            System.Threading.Thread.Sleep(25);
        }
    }

    // Presses a chord such as Ctrl+Alt+F: modifiers down, key, all up.
    public static void Chord(params ushort[] vks) {
        var list = new List<INPUT>();
        foreach (var v in vks) list.Add(Key(v, 0, 0));
        for (int i = vks.Length - 1; i >= 0; i--) list.Add(Key(vks[i], 0, KEYUP));
        SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
    }

    public static void Click(int x, int y, bool right) {
        SetCursorPos(x, y);
        System.Threading.Thread.Sleep(50);
        var down = new INPUT { type = 0 }; down.u.mi = new MOUSEINPUT { dwFlags = right ? 8u : 2u };
        var up = new INPUT { type = 0 }; up.u.mi = new MOUSEINPUT { dwFlags = right ? 16u : 4u };
        SendInput(2, new[] { down, up }, Marshal.SizeOf(typeof(INPUT)));
    }

    // Top-level windows of a process: "handle|class|visible|title|w|h".
    public static string[] Windows(int pid) {
        var found = new List<string>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid) {
                var c = new StringBuilder(256); GetClassName(h, c, 256);
                var t = new StringBuilder(512); GetWindowText(h, t, 512);
                RECT r; GetWindowRect(h, out r);
                found.Add(h.ToInt64() + "|" + c + "|" + IsWindowVisible(h) + "|" + t + "|" + (r.Right - r.Left) + "|" + (r.Bottom - r.Top));
            }
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }
}
'@

$VK = @{ CTRL = 0x11; ALT = 0x12; SHIFT = 0x10; ENTER = 0x0D; ESC = 0x1B; A = 0x41; F = 0x46; DOWN = 0x28; TAB = 0x09; BACK = 0x08 }
$AE = [Windows.Automation.AutomationElement]
$TS = [Windows.Automation.TreeScope]
$CT = [Windows.Automation.ControlType]

$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$crash = Join-Path $data 'crash.txt'
$index = Join-Path $data 'index.bin'
$rowsFile = Join-Path $data 'quick-rows.txt'
$results = New-Object System.Collections.Generic.List[object]
$shot = 0

function Record([string]$Area, [string]$Kind, [string]$Check, [bool]$Ok, [string]$Detail = '') {
    $res = if ($Ok) { 'PASS' } elseif ($Kind -eq 'core') { 'FAIL' } else { 'MISS' }
    $results.Add([pscustomobject]@{ Area = $Area; Kind = $Kind; Check = $Check; Result = $res; Detail = $Detail })
    $line = "[$res] $Area / $Check" + $(if ($Detail) { " - $Detail" } else { '' })
    if ($res -eq 'FAIL') { Write-Host "::error::$line" } elseif ($res -eq 'MISS') { Write-Host "::warning::$line" } else { Write-Host $line }
}
function Note([string]$Area, [string]$Check, [string]$Detail) {
    $results.Add([pscustomobject]@{ Area = $Area; Kind = 'note'; Check = $Check; Result = 'NOTE'; Detail = $Detail })
    Write-Host "[NOTE] $Area / $Check - $Detail"
}
function Shot([string]$Name) {
    $script:shot++
    $file = Join-Path $Out ('{0:D2}-{1}.png' -f $script:shot, $Name)
    try {
        $b = [Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
        $bmp.Save($file, [Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    } catch { Write-Host "screenshot $Name failed: $_" }
}
function Check-Crash([string]$When) {
    if (Test-Path $crash) {
        $text = (Get-Content $crash -Raw -Encoding UTF8) -replace '\s+', ' '
        Record 'stability' 'core' "no crash ($When)" $false $text.Substring(0, [Math]::Min(300, $text.Length))
        Remove-Item $crash -Force
    }
}
function Wait-Until([scriptblock]$Cond, [int]$Seconds = 10) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) {
        $v = & $Cond
        if ($v) { return $v }
        Start-Sleep -Milliseconds 250
    }
    return $null
}
function Get-Windows([int]$ProcId) {
    foreach ($w in [U]::Windows($ProcId)) {
        $p = $w.Split('|')
        [pscustomobject]@{ Handle = [IntPtr][long]$p[0]; Class = $p[1]; Visible = $p[2] -eq 'True'; Title = $p[3]; W = [int]$p[4]; H = [int]$p[5] }
    }
}
function Visible-Window([int]$ProcId, [string]$Class) {
    Get-Windows $ProcId | Where-Object { $_.Visible -and $_.Class -eq $Class } | Select-Object -First 1
}
function Main-Window([int]$ProcId) {
    Get-Windows $ProcId | Where-Object { $_.Visible -and $_.Class -notmatch '^(InstaFind|#32770|tooltips)' -and $_.W -gt 300 } |
        Sort-Object { $_.W * $_.H } -Descending | Select-Object -First 1
}
function Find-All($Root, $Type) {
    $Root.FindAll($TS::Descendants, (New-Object Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $Type)))
}
function Find-Named($Root, $Type, [string]$Like) {
    foreach ($e in (Find-All $Root $Type)) { if ($e.Current.Name -like $Like) { return $e } }
    return $null
}
function Focus-Window($Hwnd) {
    # Windows only hands the foreground to the process that last had input;
    # a tap of Alt counts as that.
    [U]::Chord($VK.ALT)
    [void][U]::SetForegroundWindow($Hwnd)
    Start-Sleep -Milliseconds 300
}
function Invoke-El($El) {
    try { $El.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke(); return $true }
    catch {
        try {
            $r = $El.Current.BoundingRectangle
            [U]::Click([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2), $false); return $true
        } catch { return $false }
    }
}
function List-Names($List) {
    $names = @()
    if ($null -eq $List) { return $names }
    foreach ($it in $List.FindAll($TS::Children, [Windows.Automation.Condition]::TrueCondition)) {
        $n = $it.Current.Name
        if ($n -and $it.Current.ControlType -ne $CT::Header) { $names += $n }
    }
    return $names
}
function Status-Text($Win) {
    $sb = Find-All $Win $CT::StatusBar | Select-Object -First 1
    if (-not $sb) { return '' }
    $t = $sb.Current.Name
    foreach ($c in $sb.FindAll($TS::Descendants, [Windows.Automation.Condition]::TrueCondition)) { if ($c.Current.Name) { $t += ' ' + $c.Current.Name } }
    return $t.Trim()
}

# ---------------------------------------------------------------- set-up
# A first start, as a new user gets it.
Get-Process bahth* -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item 'HKCU:\Software\BahthSari' -Recurse -Force -ErrorAction SilentlyContinue
$docs = Join-Path $env:USERPROFILE 'Documents\InstaFind UI test'
New-Item -ItemType Directory -Force $docs | Out-Null
Set-Content -LiteralPath (Join-Path $docs 'تقرير مالي ٢٠٢٦.txt') -Value 'الميزانية السنوية للشركة' -Encoding UTF8
Set-Content -LiteralPath (Join-Path $docs 'أحمد - سيرة ذاتية.txt') -Value 'خبرة عشر سنوات' -Encoding UTF8
Set-Content -LiteralPath (Join-Path $docs 'Quarterly Budget 2026.txt') -Value 'revenue forecast' -Encoding UTF8
Set-Content -LiteralPath (Join-Path $docs 'محاضرة الفيزياء.txt') -Value 'quantum lecture notes' -Encoding UTF8
$env:BAHTH_UI_TEST = '1'
Note 'setup' 'screen' ("{0}x{1}" -f [Windows.Forms.SystemInformation]::VirtualScreen.Width, [Windows.Forms.SystemInformation]::VirtualScreen.Height)

# ---------------------------------------------------------------- first start
$t0 = Get-Date
$p = Start-Process -FilePath $Exe -PassThru
$main = Wait-Until { Main-Window $p.Id } 20
$ms = ((Get-Date) - $t0).TotalMilliseconds
Record 'start' 'core' 'main window appears' ($null -ne $main) ("{0:N0} ms" -f $ms)
if (-not $main) { Shot 'no-window'; Get-Windows $p.Id | Format-Table | Out-String | Write-Host }
Record 'start' 'probe' 'window within 3 seconds' ($ms -lt 3000) ("{0:N0} ms" -f $ms)
Shot 'first-start'

# The one-time question about the box above the clock.
$dlg = Wait-Until { Visible-Window $p.Id '#32770' } 40
Record 'first run' 'probe' 'asks about the search box above the clock' ($null -ne $dlg)
if ($dlg) {
    $d = $AE::FromHandle($dlg.Handle)
    $texts = (Find-All $d $CT::Text | ForEach-Object { $_.Current.Name }) -join ' '
    Note 'first run' 'question text' $texts
    Shot 'first-run-question'
    $yes = $d.FindFirst($TS::Descendants, (New-Object Windows.Automation.PropertyCondition($AE::AutomationIdProperty, '6')))
    if ($yes) { [void](Invoke-El $yes) } else { [U]::Chord($VK.ENTER) }
    $box = Wait-Until { Visible-Window $p.Id 'InstaFindFloatingBox' } 10
    Record 'floating box' 'core' 'appears after answering Yes' ($null -ne $box)
    Shot 'floating-box'
}
Check-Crash 'first start'

# ---------------------------------------------------------------- index
$t1 = Get-Date
$ok = Wait-Until { (Test-Path $index) -or $p.HasExited } 1500
Record 'index' 'core' 'drives indexed and saved' ((Test-Path $index) -and -not $p.HasExited) ("{0:N0} s" -f ((Get-Date) - $t1).TotalSeconds)
Start-Sleep -Seconds 10
Check-Crash 'indexing'
if ($p.HasExited) { Record 'stability' 'core' 'program still running' $false "exit code $($p.ExitCode)" }

# ---------------------------------------------------------------- main window search
$main = Main-Window $p.Id
$win = if ($main) { $AE::FromHandle($main.Handle) } else { $null }
$query = $null; $list = $null
if ($win) {
    Note 'main window' 'title' $win.Current.Name
    $query = Find-All $win $CT::Edit | Select-Object -First 1
    $list = Find-All $win $CT::List | Select-Object -First 1
    Record 'main window' 'core' 'search box and result list found' ($null -ne $query -and $null -ne $list)
}

function Main-Search([string]$Q, [string]$Expect, [string]$Kind, [string]$Check, [int]$Wait = 15) {
    if (-not $query) { Record 'search' $Kind $Check $false 'no search box'; return }
    Focus-Window $main.Handle
    try { $query.SetFocus() } catch {}
    [U]::Chord($VK.CTRL, $VK.A); [U]::Chord($VK.BACK)
    [U]::Type($Q)
    Start-Sleep -Milliseconds 300
    $typed = try { $query.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value } catch { '' }
    if ($typed -ne $Q) {
        Note 'search' "typing '$Q'" "the box held '$typed' after typing; set it directly instead"
        try { $query.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($Q) } catch {}
    }
    [U]::Chord($VK.ENTER)
    $t = Get-Date
    $hit = Wait-Until { $n = List-Names $list; if ($Expect) { $n | Where-Object { $_ -like "*$Expect*" } | Select-Object -First 1 } else { $null } } $Wait
    $names = List-Names $list
    $status = Status-Text $win
    $top = ($names | Select-Object -First 3) -join ' | '
    if ($Expect) {
        Record 'search' $Kind $Check ($null -ne $hit) ("'$Q' -> {0:N1} s; top: {1}; status: {2}" -f ((Get-Date) - $t).TotalSeconds, $top, $status)
    } else {
        Note 'search' $Check ("'$Q' -> {0} rows; status: {1}" -f $names.Count, $status)
    }
    Shot ('search-' + ($Check -replace '[^A-Za-z0-9]+', '-'))
    Check-Crash "search '$Q'"
}

Main-Search 'تقرير مالي' 'تقرير مالي' 'core' 'Arabic file name'
Main-Search 'budget' 'Quarterly Budget' 'core' 'English file name'
Main-Search 'احمد سيرة' 'سيرة ذاتية' 'core' 'Arabic without hamza finds أحمد'
Main-Search 'تقرير ماليه' 'تقرير مالي' 'probe' 'extra letter at the end (ماليه)'
Main-Search 'hgjrvdv' 'تقرير' 'probe' 'typed on the English keyboard (hgjrvdv = التقرير)'
Main-Search 'ta2rer' 'تقرير' 'probe' 'Arabizi (ta2rer)'
Main-Search 'budgte' 'Quarterly Budget' 'probe' 'English typo (budgte)'
Main-Search 'الفيزيا' 'محاضرة الفيزياء' 'probe' 'common misspelling (الفيزيا)'
Main-Search 'zzqqxxnotthere' '' 'probe' 'no results message'

# Inside documents.
if ($win) {
    $chk = Find-All $win $CT::CheckBox | Select-Object -First 1
    if ($chk) {
        try { $chk.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle() } catch { [void](Invoke-El $chk) }
        Main-Search 'السنوية للشركة' 'تقرير مالي' 'core' 'search inside documents (Arabic text)' 60
        Main-Search 'revenue forecast' 'Quarterly Budget' 'core' 'search inside documents (English text)' 60
        try { $chk.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle() } catch { [void](Invoke-El $chk) }
    } else { Record 'search' 'core' 'search inside documents' $false 'no checkbox' }
}

# ---------------------------------------------------------------- menus
if ($win) {
    Main-Search 'تقرير مالي' 'تقرير مالي' 'core' 'Arabic file name again before the menus'
    $first = $list.FindFirst($TS::Children, (New-Object Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::ListItem)))
    if (-not $first) { $first = $list.FindFirst($TS::Children, (New-Object Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::DataItem))) }
    if ($first) {
        $r = $first.Current.BoundingRectangle
        [U]::Click([int]($r.X + 40), [int]($r.Y + $r.Height / 2), $false)
        Start-Sleep -Milliseconds 300
        [U]::Click([int]($r.X + 40), [int]($r.Y + $r.Height / 2), $true)
        $menu = Wait-Until { $m = $AE::RootElement.FindFirst($TS::Children, (New-Object Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Menu))); $m } 5
        Record 'menus' 'core' 'right-click menu on a result' ($null -ne $menu)
        if ($menu) { Note 'menus' 'right-click items' ((Find-All $menu $CT::MenuItem | ForEach-Object { $_.Current.Name }) -join ' / ') }
        Shot 'right-click-menu'
        [U]::Chord($VK.ESC); Start-Sleep -Milliseconds 300
    } else { Record 'menus' 'core' 'right-click menu on a result' $false 'no result row to click' }

    $opt = Find-Named $win $CT::Button 'Options*'
    if ($opt) {
        [void](Invoke-El $opt)
        $menu = Wait-Until { $AE::RootElement.FindFirst($TS::Children, (New-Object Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Menu))) } 5
        Record 'menus' 'core' 'Options menu opens' ($null -ne $menu)
        if ($menu) { Note 'menus' 'Options items' ((Find-All $menu $CT::MenuItem | ForEach-Object { $_.Current.Name }) -join ' / ') }
        Shot 'options-menu'
        [U]::Chord($VK.ESC); Start-Sleep -Milliseconds 300
    } else { Record 'menus' 'core' 'Options menu opens' $false 'no Options button' }
    Check-Crash 'menus'
}

# ---------------------------------------------------------------- quick search popup
function Quick-Rows {
    if (-not (Test-Path $rowsFile)) { return @() }
    try { return [IO.File]::ReadAllLines($rowsFile, [Text.Encoding]::UTF8) } catch { return @() }
}
function Quick-Search([string]$Q, [string]$Expect, [string]$Kind, [string]$Check, [int]$Wait = 10) {
    $pop = Visible-Window $p.Id 'InstaFindQuickSearch'
    if (-not $pop) { [U]::Chord($VK.CTRL, $VK.ALT, $VK.F); $pop = Wait-Until { Visible-Window $p.Id 'InstaFindQuickSearch' } 5 }
    if (-not $pop) { Record 'quick search' $Kind $Check $false 'popup not open'; return }
    [U]::Chord($VK.CTRL, $VK.A); [U]::Chord($VK.BACK)
    [U]::Type($Q)
    $t = Get-Date
    $hit = Wait-Until {
        $rows = Quick-Rows
        if ($rows.Count -gt 0 -and $rows[0] -eq "query`t$Q") { $rows | Select-Object -Skip 1 | Where-Object { $_ -like "*$Expect*" } | Select-Object -First 1 } else { $null }
    } $Wait
    $rows = Quick-Rows | Select-Object -Skip 1 | Select-Object -First 4
    Record 'quick search' $Kind $Check ($null -ne $hit) ("'$Q' -> {0:N1} s; rows: {1}" -f ((Get-Date) - $t).TotalSeconds, (($rows -replace "`t", ': ') -join ' | '))
    Shot ('quick-' + ($Check -replace '[^A-Za-z0-9]+', '-'))
    Check-Crash "quick search '$Q'"
}

[U]::Chord($VK.CTRL, $VK.ALT, $VK.F)
$pop = Wait-Until { Visible-Window $p.Id 'InstaFindQuickSearch' } 5
Record 'quick search' 'core' 'Ctrl+Alt+F opens the popup' ($null -ne $pop)
Start-Sleep -Seconds 3   # the app catalog loads in the background
Quick-Search 'notepad' 'Notepad' 'core' 'app by English name (notepad)'
Quick-Search 'تقرير مالي' 'تقرير مالي' 'core' 'Arabic file'
Quick-Search '2+2' '4' 'probe' 'calculator answer (2+2)'
Quick-Search 'غوغل كروم' 'Chrome' 'probe' 'app by Arabic name (غوغل كروم)'
Quick-Search 'كروم' 'Chrome' 'probe' 'app by short Arabic name (كروم)'
Quick-Search 'المفكرة' 'Notepad' 'probe' 'Windows app by Arabic meaning (المفكرة = Notepad)'
Quick-Search 'الة حاسبة' 'Calculator' 'probe' 'Windows app by Arabic meaning (الة حاسبة = Calculator)'
Quick-Search 'display' 'isplay' 'probe' 'Settings page (display)'
Quick-Search 'اعدادات الشاشة' 'isplay' 'probe' 'Settings page in Arabic (اعدادات الشاشة)'
Quick-Search 'notepda' 'Notepad' 'probe' 'app with a typo (notepda)'
[U]::Chord($VK.ESC)
$gone = Wait-Until { -not (Visible-Window $p.Id 'InstaFindQuickSearch') } 3
Record 'quick search' 'core' 'Esc closes the popup' ([bool]$gone)
Check-Crash 'quick search'

# ---------------------------------------------------------------- close to tray
$main = Main-Window $p.Id
if ($main) {
    $w = $AE::FromHandle($main.Handle)
    try { $w.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close() } catch { [void]$p.CloseMainWindow() }
    Start-Sleep -Seconds 3
    $p.Refresh()
    Record 'tray' 'core' 'close button hides to the tray (still running)' (-not $p.HasExited -and -not (Main-Window $p.Id))
    [U]::Chord($VK.CTRL, $VK.ALT, $VK.F)
    $back = Wait-Until { Visible-Window $p.Id 'InstaFindQuickSearch' } 5
    Record 'tray' 'core' 'Ctrl+Alt+F still works while hidden' ($null -ne $back)
    [U]::Chord($VK.ESC)
    Shot 'after-close'
}
Check-Crash 'close to tray'
if (Test-Path (Join-Path $data 'log.txt')) { Copy-Item (Join-Path $data 'log.txt') (Join-Path $Out 'log-en.txt') }
Get-Process bahth* -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

# ---------------------------------------------------------------- Arabic interface
New-Item 'HKCU:\Software\BahthSari' -Force | Out-Null
Set-ItemProperty 'HKCU:\Software\BahthSari' -Name Language -Value 'ar'
$p = Start-Process -FilePath $Exe -PassThru
$main = Wait-Until { Main-Window $p.Id } 20
Record 'arabic UI' 'core' 'starts in Arabic' ($null -ne $main -and $main.Title -match '[؀-ۿ]') $(if ($main) { $main.Title } else { '' })
Start-Sleep -Seconds 4
Shot 'arabic-main'
if ($main) {
    $win = $AE::FromHandle($main.Handle)
    $query = Find-All $win $CT::Edit | Select-Object -First 1
    $list = Find-All $win $CT::List | Select-Object -First 1
    Main-Search 'تقرير' 'تقرير مالي' 'core' 'Arabic UI search'
    $opt = Find-All $win $CT::Button | Where-Object { $_.Current.Name -like 'خيارات*' } | Select-Object -First 1
    if ($opt) { [void](Invoke-El $opt); Start-Sleep -Seconds 1; Shot 'arabic-options-menu'; [U]::Chord($VK.ESC) }
}
Quick-Search 'تقرير' 'تقرير مالي' 'core' 'Arabic UI quick search'
Quick-Search 'notepad' 'Notepad' 'probe' 'Arabic UI app search'
[U]::Chord($VK.ESC)
Check-Crash 'Arabic interface'
if (Test-Path (Join-Path $data 'log.txt')) { Copy-Item (Join-Path $data 'log.txt') (Join-Path $Out 'log-ar.txt') }
Get-Process bahth* -ErrorAction SilentlyContinue | Stop-Process -Force

# ---------------------------------------------------------------- report
$fail = @($results | Where-Object Result -eq 'FAIL').Count
$miss = @($results | Where-Object Result -eq 'MISS').Count
$pass = @($results | Where-Object Result -eq 'PASS').Count
$md = New-Object Text.StringBuilder
[void]$md.AppendLine("## Real-user UI test: $pass passed, $fail failed, $miss findings")
[void]$md.AppendLine('')
[void]$md.AppendLine('| Result | Area | Check | Detail |')
[void]$md.AppendLine('|---|---|---|---|')
foreach ($r in $results) {
    $d = ($r.Detail -replace '\|', '/' -replace '[\r\n]+', ' ')
    if ($d.Length -gt 400) { $d = $d.Substring(0, 400) + '…' }
    [void]$md.AppendLine("| $($r.Result) | $($r.Area) | $($r.Check) | $d |")
}
[void]$md.AppendLine('')
[void]$md.AppendLine('MISS = a feature we expect that did not work; it does not fail the job. Screenshots are in the ui-test artifact.')
[IO.File]::WriteAllText((Join-Path $Out 'report.md'), $md.ToString(), (New-Object Text.UTF8Encoding $false))
if ($env:GITHUB_STEP_SUMMARY) { [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $md.ToString(), (New-Object Text.UTF8Encoding $false)) }
Write-Host $md.ToString()
if ($fail -gt 0) { exit 1 }
