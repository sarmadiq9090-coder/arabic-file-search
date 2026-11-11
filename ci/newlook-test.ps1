# Gate for the new look (the WebView2 main window, src/ui/webui.rs) on a real
# Windows desktop, with the exact exe that would be published.
#
#   A. new_ui=1 in settings.txt (as after "Try the new look"): the new window
#      opens, its page loads (no blank-page fallback after the 20 s watchdog,
#      no WebView2 failure), the setting is kept, no crash, and a screenshot
#      of its client area is not one flat colour. A short search is typed
#      (screenshot only, never fails the job).
#   B. new_ui=0: the classic window opens, and its Options menu offers
#      "Try the new look".
#   C. Probe (warns, never fails): choosing that item swaps in the new window
#      without a restart and saves new_ui=1.
#   0. Before A (the default since 0.10.1): with no settings.txt at all the
#      new look opens, and the one-time switch (new_ui=1, new_ui_default=1)
#      is saved; the first-run question over it is answered No.
#   D. After C: settings as 0.10.0 left them (new_ui=0, no new_ui_default):
#      the new look opens once; "Back to the classic look" (keyboard: Ctrl+,
#      then Tab to the button) brings the classic window and saves new_ui=0;
#      after a restart the classic window stays. If the keys miss the button
#      (a probe), the test writes what the button saves and goes on.
#
# Note: in a right-to-left (mirrored) window the client origin is the top
# RIGHT corner on screen; ClientOnScreen returns the true rectangle.
#
# Output: <Out>\newlook.png, newlook-query.png, classic.png, switched.png,
# diagnostics.txt (windows, settings.txt, log.txt, crash.txt), report.md.
# Runs under Windows PowerShell 5.1 or PowerShell 7 on 64-bit Windows.
# This file is UTF-8 with a BOM so Windows PowerShell reads the Arabic in it.
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Out = (Join-Path $env:RUNNER_TEMP 'newlook-test')
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null

Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class N {
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }
    public struct RECT { public int Left, Top, Right, Bottom; }
    public struct POINT { public int X, Y; }
    delegate bool EnumProc(IntPtr h, IntPtr l);

    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, StringBuilder l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint item, StringBuilder sb, int n, uint flags);

    const uint KEYUP = 2, UNICODE = 4;
    static INPUT Key(ushort vk, ushort scan, uint flags) {
        var i = new INPUT { type = 1 };
        i.u.ki = new KEYBDINPUT { wVk = vk, wScan = scan, dwFlags = flags };
        return i;
    }
    public static void Type(string text) {
        foreach (char c in text) {
            var keys = new[] { Key(0, c, UNICODE), Key(0, c, UNICODE | KEYUP) };
            SendInput(2, keys, Marshal.SizeOf(typeof(INPUT)));
            System.Threading.Thread.Sleep(40);
        }
    }
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    static void Mouse(uint flags) {
        var i = new INPUT { type = 0 }; i.u.mi = new MOUSEINPUT { dwFlags = flags };
        SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT)));
    }
    // A real left click, press and release, at a screen point.
    public static void Click(int x, int y) {
        SetCursorPos(x, y); System.Threading.Thread.Sleep(100);
        Mouse(2); System.Threading.Thread.Sleep(60); Mouse(4);
    }
    public static void MoveTo(int x, int y) { SetCursorPos(x, y); Mouse(1); }
    public static void Down() { Mouse(2); }
    public static void Up() { Mouse(4); }
    public static void Chord(params ushort[] vks) {
        var list = new List<INPUT>();
        foreach (var v in vks) list.Add(Key(v, 0, 0));
        for (int i = vks.Length - 1; i >= 0; i--) list.Add(Key(vks[i], 0, KEYUP));
        SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
    }
    // (With a time limit: WebView2's child windows belong to its own processes.)
    public static string Text(IntPtr h) {
        var sb = new StringBuilder(1024);
        IntPtr r;
        SendMessageTimeout(h, 0x000D /* WM_GETTEXT */, (IntPtr)sb.Capacity, sb, 2 /* SMTO_ABORTIFHUNG */, 1000, out r);
        return sb.ToString();
    }
    static string Describe(IntPtr h) {
        var c = new StringBuilder(256); GetClassName(h, c, 256);
        RECT r; GetWindowRect(h, out r);
        return h.ToInt64() + "|" + c + "|" + IsWindowVisible(h) + "|" + r.Left + "|" + r.Top + "|" + (r.Right - r.Left) + "|" + (r.Bottom - r.Top) + "|" + Text(h).Replace("|", "/").Replace("\r", " ").Replace("\n", " ");
    }
    // Top-level windows of process `pid` (0: every visible one): "handle|class|visible|x|y|w|h|text".
    public static string[] Windows(int pid) {
        var found = new List<string>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (pid == 0 ? IsWindowVisible(h) : p == pid) found.Add(Describe(h));
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }
    public static string[] Children(IntPtr parent) {
        var found = new List<string>();
        EnumChildWindows(parent, (h, l) => { found.Add(Describe(h)); return true; }, IntPtr.Zero);
        return found.ToArray();
    }
    public static string[] MenuItems(IntPtr menuWindow) {
        var items = new List<string>();
        IntPtr m = SendMessage(menuWindow, 0x01E1 /* MN_GETHMENU */, IntPtr.Zero, IntPtr.Zero);
        int n = GetMenuItemCount(m);
        for (int i = 0; i < n; i++) {
            var sb = new StringBuilder(512);
            GetMenuString(m, (uint)i, sb, 512, 0x400 /* MF_BYPOSITION */);
            items.Add(sb.ToString().Replace("\t", "  "));
        }
        return items.ToArray();
    }
    // The client area in screen pixels: x, y, width, height.
    public static int[] ClientOnScreen(IntPtr h) {
        // Both corners: in a right-to-left (mirrored) window the client
        // origin is the top RIGHT corner on screen.
        RECT r; GetClientRect(h, out r);
        var a = new POINT(); a.X = r.Left; a.Y = r.Top;
        var b = new POINT(); b.X = r.Right; b.Y = r.Bottom;
        ClientToScreen(h, ref a);
        ClientToScreen(h, ref b);
        return new[] { Math.Min(a.X, b.X), Math.Min(a.Y, b.Y), Math.Abs(b.X - a.X), Math.Abs(b.Y - a.Y) };
    }
}
'@
# Real pixels, whatever the display scale.
[void][N]::SetProcessDPIAware()

$VK = @{ ALT = 0x12; ENTER = 0x0D; ESC = 0x1B; DOWN = 0x28; CTRL = 0x11; A = 0x41; TAB = 0x09; COMMA = 0xBC; SPACE = 0x20 }
$BM_CLICK = 0x00F5
$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$settingsFile = Join-Path $data 'settings.txt'
$logFile = Join-Path $data 'log.txt'
$crash = Join-Path $data 'crash.txt'
$results = New-Object System.Collections.Generic.List[object]
$script:proc = $null

function Say([string]$Text) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $Text) }
function Record([string]$Kind, [string]$Check, [bool]$Ok, [string]$Detail = '') {
    $res = if ($Ok) { 'PASS' } elseif ($Kind -eq 'core') { 'FAIL' } else { 'MISS' }
    $results.Add([pscustomobject]@{ Kind = $Kind; Check = $Check; Result = $res; Detail = $Detail })
    $line = "[$res] $Check" + $(if ($Detail) { " - $Detail" } else { '' })
    if ($res -eq 'FAIL') { Write-Host "::error::$line" } elseif ($res -eq 'MISS') { Write-Host "::warning::$line" } else { Write-Host $line }
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
function Parse-Win([string]$Line) {
    $p = $Line.Split('|', 8)
    [pscustomobject]@{ Handle = [IntPtr][long]$p[0]; Class = $p[1]; Visible = $p[2] -eq 'True'; X = [int]$p[3]; Y = [int]$p[4]; W = [int]$p[5]; H = [int]$p[6]; Text = $p[7] }
}
function Get-Windows([int]$ProcId) { foreach ($l in [N]::Windows($ProcId)) { Parse-Win $l } }
function Get-Children($Hwnd) { foreach ($l in [N]::Children($Hwnd)) { Parse-Win $l } }
# The program's main windows (nwg's class), visible and big.
function Main-Windows([int]$ProcId) {
    @(Get-Windows $ProcId | Where-Object { $_.Visible -and $_.Class -eq 'NativeWindowsGuiWindow' -and $_.W -gt 300 -and $_.H -gt 200 })
}
# The classic window holds the result list; the new one holds WebView2.
function Is-Classic($W) { [bool](Get-Children $W.Handle | Where-Object Class -eq 'SysListView32') }
function New-Look([int]$ProcId) {
    Main-Windows $ProcId | Where-Object { $_.Text -notmatch '\((classic|new)\)' -and -not (Is-Classic $_) } | Select-Object -First 1
}
function Classic([int]$ProcId) {
    Main-Windows $ProcId | Where-Object { Is-Classic $_ } | Select-Object -First 1
}
function Open-Menu { Get-Windows 0 | Where-Object { $_.Class -eq '#32768' } | Select-Object -First 1 }
function Focus-Window($Hwnd) {
    # Windows only hands the foreground to the process that last had input;
    # a tap of Alt counts as that.
    [N]::Chord($VK.ALT)
    [void][N]::SetForegroundWindow($Hwnd)
    Start-Sleep -Milliseconds 400
}
function Read-Settings { if (Test-Path $settingsFile) { [IO.File]::ReadAllText($settingsFile) } else { '' } }
function Setting([string]$Text, [string]$Key) {
    $m = [regex]::Matches($Text, "(?m)^\s*$Key\s*=\s*(.*?)\s*$")
    if ($m.Count) { $m[$m.Count - 1].Groups[1].Value } else { $null }
}
# settings.txt as the program reads it (key=value, ASCII, no BOM): no
# first-run question (box above the clock, moving the exe, Store rating).
# With -Old: as 0.10.0 wrote it, without the one-time default marker
# (new_ui_default), so the program makes the new look the default once.
function Write-Settings([int]$NewUi, [switch]$Old) {
    New-Item -ItemType Directory -Force $data | Out-Null
    $text = "floating_asked=1`nmove_asked=1`nrating_asked=1`nnew_ui=$NewUi`nnew_ui_failed=0`n"
    if (-not $Old) { $text += "new_ui_default=1`n" }
    [IO.File]::WriteAllText($settingsFile, $text, (New-Object Text.ASCIIEncoding))
    Say "settings.txt written: $($text -replace "`n", ' ')"
}
# Answers the program's message boxes (the first-run questions) with No,
# else Cancel, else closes them. Returns how many there were.
function Dismiss-Dialogs([int]$ProcId) {
    $n = 0
    foreach ($d in @(Get-Windows $ProcId | Where-Object { $_.Visible -and $_.Class -eq '#32770' })) {
        $n++
        $text = (Get-Children $d.Handle | Where-Object { $_.Class -eq 'Static' -and $_.Text } | ForEach-Object Text) -join ' '
        Say "dialog '$($d.Text)': $text"
        $no = [N]::GetDlgItem($d.Handle, 7)
        if ($no -eq [IntPtr]::Zero) { $no = [N]::GetDlgItem($d.Handle, 2) }
        if ($no -ne [IntPtr]::Zero) { [void][N]::PostMessage($no, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero) }
        else { [void][N]::PostMessage($d.Handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }  # WM_CLOSE
    }
    return $n
}
# The page's elements, as Windows UI Automation sees them (WebView2 exposes
# the page's accessibility tree): the first one of `ControlType` whose name
# matches `Pattern`, with a non-empty rectangle on screen.
function Find-Uia($Hwnd, $ControlType, [string]$Pattern) {
    try { return Find-UiaIn ([Windows.Automation.AutomationElement]::FromHandle($Hwnd)) $ControlType $Pattern } catch { Say "UI Automation: $_"; return $null }
}
function Find-UiaIn($Root, $ControlType, [string]$Pattern) {
    if (-not $Root) { return $null }
    try {
        $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, $ControlType)
        foreach ($e in $root.FindAll([Windows.Automation.TreeScope]::Descendants, $cond)) {
            $r = $e.Current.BoundingRectangle
            if ($e.Current.Name -match $Pattern -and -not $r.IsEmpty -and $r.Width -gt 2 -and $r.Height -gt 2) { return $e }
        }
    } catch { Say "UI Automation: $_" }
    return $null
}
function Uia-Names($Hwnd, $ControlType) {
    try {
        $root = [Windows.Automation.AutomationElement]::FromHandle($Hwnd)
        $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, $ControlType)
        return @($root.FindAll([Windows.Automation.TreeScope]::Descendants, $cond) | ForEach-Object { $_.Current.Name } | Select-Object -First 40)
    } catch { return @("(UI Automation failed: $_)") }
}
function Center($El) {
    $r = $El.Current.BoundingRectangle
    return @([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2))
}
function Is-Selected($El) {
    try { return $El.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Current.IsSelected } catch { return $null }
}
function Log-Text { if (Test-Path $logFile) { [IO.File]::ReadAllText($logFile, [Text.Encoding]::UTF8) } else { '' } }
function Log-Lines([string]$Pattern) { @((Log-Text) -split "`r?`n" | Where-Object { $_ -match $Pattern }) }

# Saves the window's client area to <Out>\<Name>.png; returns how varied it
# is: distinct colours among a grid of samples, and the share of the most
# common one. A page that never painted is one flat colour.
function Shot-Client($W, [string]$Name) {
    $file = Join-Path $Out "$Name.png"
    try {
        $c = [N]::ClientOnScreen($W.Handle)
        if ($c[2] -lt 50 -or $c[3] -lt 50) { Say "screenshot ${Name}: client area too small ($($c -join ','))"; return $null }
        $bmp = New-Object Drawing.Bitmap $c[2], $c[3]
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($c[0], $c[1], 0, 0, $bmp.Size)
        $bmp.Save($file, [Drawing.Imaging.ImageFormat]::Png)
        $counts = @{}
        $n = 0
        for ($y = 2; $y -lt $c[3]; $y += 7) {
            for ($x = 2; $x -lt $c[2]; $x += 7) {
                $k = $bmp.GetPixel($x, $y).ToArgb()
                $counts[$k] = 1 + [int]$counts[$k]
                $n++
            }
        }
        $g.Dispose(); $bmp.Dispose()
        $top = ($counts.Values | Measure-Object -Maximum).Maximum
        $r = [pscustomobject]@{ File = $file; Colours = $counts.Count; Top = [Math]::Round($top / [Math]::Max(1, $n), 3); Samples = $n }
        Say ("screenshot {0}: {1}x{2} at {3},{4}; {5} colours in {6} samples, most common {7:P1}" -f $Name, $c[2], $c[3], $c[0], $c[1], $r.Colours, $r.Samples, $r.Top)
        return $r
    } catch { Say "screenshot $Name failed: $_"; return $null }
}
function Full-Shot([string]$Name) {
    try {
        $b = [Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
        $bmp.Save((Join-Path $Out "$Name.png"), [Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    } catch { Say "screenshot $Name failed: $_" }
}
function Diagnostics([string]$When) {
    $f = Join-Path $Out 'diagnostics.txt'
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("==== $When ($(Get-Date -Format s))")
    if ($script:proc) {
        $script:proc.Refresh()
        $lines.Add("process $($script:proc.Id): exited=$($script:proc.HasExited)" + $(if ($script:proc.HasExited) { " code=$($script:proc.ExitCode)" } else { '' }))
        if (-not $script:proc.HasExited) {
            foreach ($w in Get-Windows $script:proc.Id) {
                $lines.Add("window $($w.Handle) $($w.Class) visible=$($w.Visible) $($w.X),$($w.Y) $($w.W)x$($w.H) '$($w.Text)'")
                if ($w.Visible) { foreach ($k in Get-Children $w.Handle) { $lines.Add("    child $($k.Class) visible=$($k.Visible) $($k.W)x$($k.H) '$($k.Text)'") } }
            }
        }
    }
    $lines.Add('-- WebView2 processes:')
    foreach ($q in @(Get-Process msedgewebview2 -ErrorAction SilentlyContinue)) { $lines.Add("    pid $($q.Id)") }
    $lines.Add('-- settings.txt:'); $lines.Add((Read-Settings))
    $lines.Add('-- log.txt (last 60 lines):')
    foreach ($l in @((Log-Text) -split "`r?`n" | Select-Object -Last 60)) { $lines.Add($l) }
    if (Test-Path $crash) { $lines.Add('-- crash.txt:'); $lines.Add([IO.File]::ReadAllText($crash, [Text.Encoding]::UTF8)) }
    $text = ($lines -join "`r`n") + "`r`n"
    [IO.File]::AppendAllText($f, $text, [Text.Encoding]::UTF8)
    Write-Host $text
}
function Stop-Program {
    Get-Process bahth* -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    if ($script:proc) { [void]$script:proc.WaitForExit(10000) }
    # Its WebView2 processes follow it; give them a moment.
    [void](Wait-Until { -not (Get-Process msedgewebview2 -ErrorAction SilentlyContinue) } 10)
    Start-Sleep -Seconds 1
}
function Start-Program {
    Say "starting $Exe"
    $script:proc = Start-Process -FilePath $Exe -PassThru
    Say "process $($script:proc.Id)"
}
function Check-Crash([string]$When) {
    $ok = -not (Test-Path $crash)
    $detail = ''
    if (-not $ok) { $t = [IO.File]::ReadAllText($crash, [Text.Encoding]::UTF8) -replace '\s+', ' '; $detail = $t.Substring(0, [Math]::Min(300, $t.Length)) }
    Record 'core' "no crash report ($When)" $ok $detail
}
function Alive([string]$When) {
    $script:proc.Refresh()
    Record 'core' "program still running ($When)" (-not $script:proc.HasExited) $(if ($script:proc.HasExited) { "exit code $($script:proc.ExitCode)" } else { '' })
}

# ---------------------------------------------------------------- set-up
Say ("PowerShell {0}, screen {1}x{2}, Windows {3}" -f $PSVersionTable.PSVersion, [Windows.Forms.SystemInformation]::VirtualScreen.Width, [Windows.Forms.SystemInformation]::VirtualScreen.Height, [Environment]::OSVersion.VersionString)
$pe = [IO.File]::ReadAllBytes($Exe)
Say ("exe {0}: {1:N0} bytes, PE machine {2:x4}" -f $Exe, $pe.Length, [BitConverter]::ToUInt16($pe, [BitConverter]::ToInt32($pe, 0x3c) + 4))

# The WebView2 runtime the new look needs, as its installer registers it.
$wv2 = $null
$key = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
foreach ($k in @("HKLM:\SOFTWARE\WOW6432Node\$key", "HKLM:\SOFTWARE\$key", "HKCU:\Software\$key")) {
    $v = (Get-ItemProperty -Path $k -Name pv -ErrorAction SilentlyContinue).pv
    Say "WebView2 runtime at ${k}: $(if ($v) { $v } else { '(none)' })"
    if ($v -and $v -ne '0.0.0.0' -and -not $wv2) { $wv2 = $v }
}
foreach ($dir in @("${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application", "$env:ProgramFiles\Microsoft\EdgeWebView\Application")) {
    if (Test-Path $dir) {
        $vers = @(Get-ChildItem $dir -Directory -ErrorAction SilentlyContinue | Where-Object Name -match '^\d+\.\d+' | ForEach-Object Name)
        Say "WebView2 folder ${dir}: $($vers -join ', ')"
        if (-not $wv2 -and $vers.Count) { $wv2 = "$($vers[-1]) (folder only, not registered)" }
    }
}
if (-not $wv2) {
    Write-Host '::error::No WebView2 runtime on this runner, so the new look cannot be tested here. Install it in the job (Evergreen bootstrapper) or use a runner image that has it.'
    exit 1
}
Say "WebView2 runtime $wv2"

# A clean start, as a new user.
Get-Process bahth* -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item 'HKCU:\Software\BahthSari' -Recurse -Force -ErrorAction SilentlyContinue

try {
    # ------------------------------------------------------------ 0. a new user: the default
    Say 'part 0: no settings.txt at all'
    $t0 = Get-Date
    Start-Program
    $win = Wait-Until { New-Look $script:proc.Id } 40
    Record 'core' 'a new user (no settings.txt) gets the new look' ($null -ne $win) $(if ($win) { "'$($win.Text)' after {0:N1} s" -f ((Get-Date) - $t0).TotalSeconds } else { 'no new window' })
    # The first-run question about the box above the clock comes over it.
    $asked = Wait-Until { @(Get-Windows $script:proc.Id | Where-Object { $_.Visible -and $_.Class -eq '#32770' }).Count -gt 0 } 20
    Record 'probe' 'the first-run question shows over the new look' ([bool]$asked)
    if ($asked) { Full-Shot 'first-run-question'; [void](Dismiss-Dialogs $script:proc.Id); Start-Sleep -Seconds 2 }
    $s = Read-Settings
    Record 'core' 'the one-time default is saved (new_ui=1, new_ui_default=1)' (((Setting $s 'new_ui') -eq '1') -and ((Setting $s 'new_ui_default') -eq '1')) ("new_ui=$(Setting $s 'new_ui') new_ui_default=$(Setting $s 'new_ui_default')")
    Record 'core' 'the classic window stays hidden (new user)' ($null -eq (Classic $script:proc.Id))
    Alive 'new user'
    Check-Crash 'new user'
    if ($results | Where-Object Result -eq 'FAIL') { Diagnostics 'new user' }
    Stop-Program
    Write-Settings 1

    # ------------------------------------------------------------ A. the new look
    $t0 = Get-Date
    Start-Program
    $win = Wait-Until { New-Look $script:proc.Id } 40
    Record 'core' 'the new look opens with new_ui=1' ($null -ne $win) $(if ($win) { "'$($win.Text)' $($win.W)x$($win.H) after {0:N1} s" -f ((Get-Date) - $t0).TotalSeconds } else { 'no visible window without a result list' })
    foreach ($l in Log-Lines 'webui') { Say "log: $l" }
    if ($win) {
        $kids = @(Get-Children $win.Handle)
        Say ("new window's children: " + (($kids | ForEach-Object { $_.Class } | Select-Object -Unique) -join ', '))
        Record 'probe' 'WebView2 inside the new window' ([bool]($kids | Where-Object { $_.Class -match '^Chrome_' })) (($kids | ForEach-Object Class | Select-Object -Unique) -join ', ')
        $classicShown = Classic $script:proc.Id
        Record 'core' 'the classic window stays hidden' ($null -eq $classicShown) $(if ($classicShown) { "'$($classicShown.Text)'" } else { '' })
    }
    # Past the blank-page watchdog (20 s after the page is created) and its
    # one retry: a page that never said hello would have been reloaded or
    # given up by then, and the log would say so.
    $wait = 45 - ((Get-Date) - $t0).TotalSeconds
    if ($wait -gt 0) { Say ("waiting {0:N0} s past the blank-page watchdog" -f $wait); Start-Sleep -Seconds ([int][Math]::Ceiling($wait)) }
    Alive 'new look, after 45 s'
    Check-Crash 'new look'
    foreach ($l in Log-Lines 'webui') { Say "log: $l" }
    $bad = @(Log-Lines 'webui: .*(did not load|process failed|showing the page again|back to the classic look|using the classic window|refused a message)')
    $retried = @(Log-Lines 'webui: first try failed')
    if ($retried.Count) { Record 'probe' 'WebView2 started at the first try' $false ($retried -join ' | ') }
    Record 'core' 'the page loaded: no WebView2 failure or blank-page fallback in log.txt' ($bad.Count -eq 0) ($bad -join ' | ')
    Record 'core' 'log.txt says the new window is ready' ((Log-Lines 'webui: main window ready').Count -gt 0)
    $s = Read-Settings
    Record 'core' 'settings.txt keeps new_ui=1' ((Setting $s 'new_ui') -eq '1') ("new_ui=" + (Setting $s 'new_ui'))
    Record 'core' 'settings.txt has no new_ui_failed=1' ((Setting $s 'new_ui_failed') -ne '1') ("new_ui_failed=" + (Setting $s 'new_ui_failed'))
    $win = New-Look $script:proc.Id
    Record 'core' 'the new look is still the window on screen' ($null -ne $win)
    if ($win) {
        Focus-Window $win.Handle
        $shot = Shot-Client $win 'newlook'
        Record 'core' 'the new window is painted (not one flat colour)' ($shot -and $shot.Colours -ge 8 -and $shot.Top -lt 0.97) $(if ($shot) { "$($shot.Colours) colours, most common $($shot.Top)" } else { 'no screenshot' })
        # A short search, for the screenshot only: the page puts the focus
        # in its search box when shown.
        try {
            [N]::Type('report')
            Start-Sleep -Seconds 3
            $q = Shot-Client $win 'newlook-query'
            Record 'probe' 'typing changes the page' ($q -and $shot -and ($q.Colours -ne $shot.Colours -or $q.Top -ne $shot.Top)) $(if ($q) { "$($q.Colours) colours, most common $($q.Top)" } else { '' })
        } catch { Record 'probe' 'typing a search' $false "$_" }
        Alive 'after typing'
        Check-Crash 'after typing'

        # A plain click on a result selects it and never starts a drag; a
        # press that moves starts one only while the button is down (the
        # program's own drag of the file, dropped back on its own search
        # box, which takes nothing). Results come in while the drives are
        # indexed: search for something every Windows has.
        [N]::Chord($VK.CTRL, 0x4B)  # Ctrl+K: the search box, text selected
        Start-Sleep -Milliseconds 300
        [N]::Type('notepad')
        $rowIn = { Find-UiaIn (Find-Uia $win.Handle ([Windows.Automation.ControlType]::List) '^(Results|النتائج)$') ([Windows.Automation.ControlType]::ListItem) '\S' }
        $row = Wait-Until $rowIn 60
        if (-not $row) {
            Record 'probe' 'a result row to click (UI Automation)' $false ("list items: " + ((Uia-Names $win.Handle ([Windows.Automation.ControlType]::ListItem)) -join ' | '))
        } else {
            $drags = (Log-Lines 'webui: drag').Count
            $c = Center $row
            Say "clicking result '$($row.Current.Name)' at $($c[0]),$($c[1])"
            [N]::Click($c[0], $c[1])
            Start-Sleep -Seconds 1
            Record 'core' 'a plain click on a result starts no drag' ((Log-Lines 'webui: drag').Count -eq $drags) (((Log-Lines 'webui: drag') | Select-Object -Last 2) -join ' | ')
            $sel = Is-Selected $row
            if ($null -eq $sel) { Record 'probe' 'the clicked result is selected (UI Automation SelectionItem)' $false 'pattern not available' }
            else { Record 'core' 'the clicked result is selected (the click is not swallowed)' ([bool]$sel) }
            [void](Shot-Client $win 'row-clicked')
            # A real drag, onto the page's own search box.
            $box = Find-Uia $win.Handle ([Windows.Automation.ControlType]::ComboBox) '.'
            $row = & $rowIn
            if ($row -and $box) {
                $c = Center $row; $b = Center $box
                [N]::MoveTo($c[0], $c[1]); Start-Sleep -Milliseconds 200
                [N]::Down(); Start-Sleep -Milliseconds 200
                for ($k = 1; $k -le 5; $k++) { [N]::MoveTo($c[0] + 4 * $k, $c[1] + 2 * $k); Start-Sleep -Milliseconds 80 }
                Start-Sleep -Milliseconds 600
                for ($k = 1; $k -le 8; $k++) { [N]::MoveTo([int]($c[0] + ($b[0] - $c[0]) * $k / 8), [int]($c[1] + ($b[1] - $c[1]) * $k / 8)); Start-Sleep -Milliseconds 80 }
                Start-Sleep -Milliseconds 300
                [N]::Up()
                Start-Sleep -Seconds 2
                $started = @(Log-Lines 'webui: drag started').Count -gt 0
                Record 'probe' 'dragging a result starts the program''s drag' $started (((Log-Lines 'webui: drag') | Select-Object -Last 2) -join ' | ')
                Alive 'after a drag'
                Check-Crash 'after a drag'
                [void](Shot-Client $win 'after-drag')
            } else {
                Record 'probe' 'a result and the search box to drag between (UI Automation)' $false
            }
        }
    } else {
        Full-Shot 'newlook-screen'
    }
    if ($results | Where-Object Result -eq 'FAIL') { Diagnostics 'new look' }
    Stop-Program

    # ------------------------------------------------------------ B. the classic window
    Write-Settings 0
    $t0 = Get-Date
    Start-Program
    $cw = Wait-Until { Classic $script:proc.Id } 40
    Record 'core' 'the classic window opens with new_ui=0' ($null -ne $cw) $(if ($cw) { "'$($cw.Text)' after {0:N1} s" -f ((Get-Date) - $t0).TotalSeconds } else { '' })
    $other = New-Look $script:proc.Id
    Record 'core' 'no new window with new_ui=0' ($null -eq $other)
    if ($cw) {
        Start-Sleep -Seconds 3
        Focus-Window $cw.Handle
        [void](Shot-Client $cw 'classic')
        # The Options menu offers the new look (first item).
        $options = Get-Children $cw.Handle | Where-Object { $_.Visible -and $_.Class -eq 'Button' -and $_.Text -match '^Options|^خيارات' } | Select-Object -First 1
        $items = @()
        $menu = $null
        if ($options) {
            # Focus-Window's Alt tap can leave the window in menu mode, where
            # a popup menu closes at once: leave that mode first.
            [void][N]::PostMessage($cw.Handle, 0x001F, [IntPtr]::Zero, [IntPtr]::Zero)  # WM_CANCELMODE
            Start-Sleep -Milliseconds 500
            for ($try = 1; $try -le 3 -and -not $menu; $try++) {
                [void][N]::PostMessage($options.Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
                $menu = Wait-Until { Open-Menu } 6
            }
            if ($menu) { $items = @([N]::MenuItems($menu.Handle)); Say ("Options items: " + ($items -join ' / ')) }
        }
        if ($options -and -not $menu) { Say 'no popup menu window (#32768) after clicking Options'; Diagnostics 'options menu did not open' }
        $first = if ($items.Count) { $items[0] } else { '' }
        Record 'core' 'the Options menu offers "Try the new look"' ($first -match 'Try the new look|جرّب الشكل الجديد') $(if ($options) { "first item '$first'" } else { 'no Options button' })
        Full-Shot 'classic-options'

        # ------------------------------------------------------------ C. probe: switch without a restart
        if ($menu -and $first -match 'Try the new look|جرّب الشكل الجديد') {
            try {
                # (The classic window has the foreground, so the open menu
                # takes the keys; no Alt tap here, it would close the menu.)
                [N]::Chord($VK.DOWN)
                Start-Sleep -Milliseconds 300
                [N]::Chord($VK.ENTER)
                $sw = Wait-Until { New-Look $script:proc.Id } 40
                Record 'probe' '"Try the new look" swaps in the new window without a restart' ($null -ne $sw) $(if ($sw) { "'$($sw.Text)'" } else { '' })
                if ($sw) {
                    Start-Sleep -Seconds 25   # past the blank-page watchdog
                    $sw = New-Look $script:proc.Id
                    if ($sw) { Focus-Window $sw.Handle; $x = Shot-Client $sw 'switched'; Record 'probe' 'the switched-in page is painted' ($x -and $x.Colours -ge 8 -and $x.Top -lt 0.97) $(if ($x) { "$($x.Colours) colours" } else { '' }) }
                    Record 'probe' 'new_ui=1 saved after switching' ((Setting (Read-Settings) 'new_ui') -eq '1') ("new_ui=" + (Setting (Read-Settings) 'new_ui'))
                    $back = Classic $script:proc.Id
                    Record 'probe' 'the classic window is hidden after switching' ($null -eq $back)
                } else {
                    Full-Shot 'switch-screen'
                }
            } catch { Record 'probe' 'switching from the Options menu' $false "$_" }
        } elseif ($menu) {
            [N]::Chord($VK.ESC)
        }
        Alive 'classic and switch'
        Check-Crash 'classic and switch'
    } else {
        Full-Shot 'classic-screen'
    }
    Stop-Program

    # ------------------------------------------------------------ D. 0.10.0 settings: the new look once, then the user's choice
    Say 'part D: settings from 0.10.0 (new_ui=0, no new_ui_default)'
    Write-Settings 0 -Old
    $t0 = Get-Date
    Start-Program
    $win = Wait-Until { New-Look $script:proc.Id } 40
    Record 'core' '0.10.0 settings with new_ui=0: the new look opens once' ($null -ne $win) $(if ($win) { "after {0:N1} s" -f ((Get-Date) - $t0).TotalSeconds } else { 'no new window' })
    $s = Read-Settings
    Record 'core' 'the one-time switch is saved (new_ui=1, new_ui_default=1)' (((Setting $s 'new_ui') -eq '1') -and ((Setting $s 'new_ui_default') -eq '1')) ("new_ui=$(Setting $s 'new_ui') new_ui_default=$(Setting $s 'new_ui_default')")
    $clicked = $false
    if ($win) {
        # Let the page load, open its settings (the gear button, found with
        # UI Automation, else Ctrl+,), then click "Back to the classic look"
        # with the mouse where UI Automation says it is.
        Start-Sleep -Seconds 8
        Focus-Window $win.Handle
        [void][N]::PostMessage($win.Handle, 0x001F, [IntPtr]::Zero, [IntPtr]::Zero)  # WM_CANCELMODE (the Alt tap)
        Start-Sleep -Milliseconds 300
        $gear = Find-Uia $win.Handle ([Windows.Automation.ControlType]::Button) '^(Settings|الإعدادات)'
        if ($gear) { $c = Center $gear; [N]::Click($c[0], $c[1]) } else { Say 'no Settings button in UI Automation; pressing Ctrl+,'; [N]::Chord($VK.CTRL, $VK.COMMA) }
        $back = Wait-Until { Find-Uia $win.Handle ([Windows.Automation.ControlType]::Button) '^(Back to the classic look|العودة إلى الشكل القديم)$' } 10
        if (-not $back) { Say ("buttons UI Automation sees: " + ((Uia-Names $win.Handle ([Windows.Automation.ControlType]::Button)) -join ' | ')) }
        Record 'core' 'the "Back to the classic look" button is on the settings page' ($null -ne $back)
        [void](Shot-Client $win 'settings-page')
        if ($back) {
            $c = Center $back
            Say "clicking 'Back to the classic look' at $($c[0]),$($c[1])"
            [N]::Click($c[0], $c[1])
        }
        $cw = Wait-Until { Classic $script:proc.Id } 10
        $clicked = $null -ne $cw
        Record 'core' 'clicking "Back to the classic look" brings the classic window' $clicked
        if ($clicked) {
            Start-Sleep -Seconds 2
            $s = Read-Settings
            Record 'core' '"Back to the classic look" saves new_ui=0 (the marker kept)' (((Setting $s 'new_ui') -eq '0') -and ((Setting $s 'new_ui_default') -eq '1')) ("new_ui=$(Setting $s 'new_ui') new_ui_default=$(Setting $s 'new_ui_default')")
            Record 'core' 'the new window is gone after going back' ($null -eq (New-Look $script:proc.Id))
            [void](Shot-Client $cw 'back-to-classic')
        }
    }
    Alive '0.10.0 settings'
    Check-Crash '0.10.0 settings'
    Stop-Program
    if (-not $clicked) {
        # (Already a failure above; the restart check still runs on what
        # the button saves.)
        Write-Settings 0
    }
    # A restart: the user's choice sticks, the one-time switch is not made again.
    Start-Program
    $cw = Wait-Until { Classic $script:proc.Id } 40
    Record 'core' 'after "Back to the classic look" and a restart, the classic window opens' ($null -ne $cw)
    Record 'core' 'and no new window' ($null -eq (New-Look $script:proc.Id))
    Start-Sleep -Seconds 3
    $s = Read-Settings
    Record 'core' 'settings.txt still says new_ui=0 after the restart' ((Setting $s 'new_ui') -eq '0') ("new_ui=$(Setting $s 'new_ui') new_ui_default=$(Setting $s 'new_ui_default')")
    if ($cw) { [void](Shot-Client $cw 'classic-after-restart') }
    Alive 'classic after restart'
    Check-Crash 'classic after restart'
} catch {
    Record 'core' 'the test ran to the end' $false "$_ at line $($_.InvocationInfo.ScriptLineNumber)"
} finally {
    $failed = [bool]($results | Where-Object Result -eq 'FAIL')
    Diagnostics $(if ($failed) { 'end (failed)' } else { 'end' })
    Stop-Program
}

# ---------------------------------------------------------------- report
$md = New-Object System.Text.StringBuilder
[void]$md.AppendLine('## New look (WebView2) on real Windows')
[void]$md.AppendLine('')
[void]$md.AppendLine("WebView2 runtime $wv2")
[void]$md.AppendLine('')
[void]$md.AppendLine('| Result | Kind | Check | Detail |')
[void]$md.AppendLine('|---|---|---|---|')
foreach ($r in $results) { [void]$md.AppendLine("| $($r.Result) | $($r.Kind) | $($r.Check) | $(($r.Detail -replace '\|', '/'))  |") }
[IO.File]::WriteAllText((Join-Path $Out 'report.md'), $md.ToString(), (New-Object Text.UTF8Encoding $true))
if ($env:GITHUB_STEP_SUMMARY) { Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value $md.ToString() -Encoding UTF8 }
$fails = @($results | Where-Object Result -eq 'FAIL')
Say ("{0} checks: {1} failed, {2} missed probes" -f $results.Count, $fails.Count, @($results | Where-Object Result -eq 'MISS').Count)
if ($fails.Count) { exit 1 }
Say 'New-look test passed.'
