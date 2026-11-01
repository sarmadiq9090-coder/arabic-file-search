# Screenshots and checks of the quick search (0.8) on a real Windows machine:
# the floating box above the taskbar (idle and hovered), the Ctrl+Alt+F popup
# empty and with typed queries, the Tab action menu and the tray area, in
# Arabic and English and once with the light theme. Fails on a crash, an
# early exit, a popup that does not open or close, or queries that show
# nothing. Then compares the command-line search and the start-up of this exe
# with InstaFind 0.6.1 on the same index. Everything goes to $Out.
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out,
    [string]$OldExe
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$index = Join-Path $data 'index.bin'
$crash = Join-Path $data 'crash.txt'
$logFile = Join-Path $data 'log.txt'
$script:failed = @()
function Fail([string]$msg) { Write-Host "::error::$msg"; $script:failed += $msg }
function Note([string]$msg) { Write-Host $msg; Add-Content -LiteralPath (Join-Path $Out 'checks.txt') -Value $msg -Encoding utf8 }

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
public static class Q {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit, Size = 40)] struct INPUT {
        [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; [FieldOffset(8)] public MOUSEINPUT mi;
    }
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    public static string Title(IntPtr h) { var s = new StringBuilder(256); GetWindowText(h, s, 256); return s.ToString(); }
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, string l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, string l, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, uint rop);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    public static int ScreenW { get { return GetSystemMetrics(0); } }
    public static int ScreenH { get { return GetSystemMetrics(1); } }
    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
    // The first top-level window of `cls` (of process `pid`, 0 = any), visible or not.
    public static IntPtr Find(int pid, string cls) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if ((pid == 0 || p == pid) && Class(h) == cls) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static IntPtr FindVisible(string cls) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            if (IsWindowVisible(h) && Class(h) == cls) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static IntPtr MainWindow(int pid) {
        IntPtr best = IntPtr.Zero; long area = 0;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h)) {
                RECT r; GetWindowRect(h, out r);
                long a = (long)(r.R - r.L) * (r.B - r.T);
                if (a > area) { area = a; best = h; }
            }
            return true;
        }, IntPtr.Zero);
        return best;
    }
    public static IntPtr Child(IntPtr parent, string cls) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(parent, (h, l) => { if (Class(h) == cls) { found = h; return false; } return true; }, IntPtr.Zero);
        return found;
    }
    public static RECT Rect(IntPtr h) {
        RECT r;
        if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) != 0) GetWindowRect(h, out r);
        return r;
    }
    public static void SetText(IntPtr h, string t) { SendMessage(h, 0x000C, IntPtr.Zero, t); }
    // WM_SETTINGCHANGE "ImmersiveColorSet", as Settings sends after a theme change.
    public static void BroadcastTheme() {
        IntPtr r; SendMessageTimeout((IntPtr)0xFFFF, 0x001A, IntPtr.Zero, "ImmersiveColorSet", 2, 3000, out r);
    }
    static INPUT Key(ushort vk, ushort scan, uint flags) {
        var i = new INPUT(); i.type = 1; i.ki.vk = vk; i.ki.scan = scan; i.ki.flags = flags; return i;
    }
    static INPUT Mouse(uint flags) { var i = new INPUT(); i.type = 0; i.mi.flags = flags; return i; }
    public static void Char(char c) {
        SendInput(2, new[] { Key(0, c, 4), Key(0, c, 4 | 2) }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void VKey(ushort vk) {
        SendInput(2, new[] { Key(vk, 0, 0), Key(vk, 0, 2) }, Marshal.SizeOf(typeof(INPUT)));
    }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
    // The class of the top-level window that a click at (x, y) would reach.
    public static string ClassAt(int x, int y) {
        var p = new POINT(); p.X = x; p.Y = y;
        return Class(GetAncestor(WindowFromPoint(p), 2));
    }
    public static void Combo(ushort[] mods, ushort vk) {
        var list = new List<INPUT>();
        foreach (var m in mods) list.Add(Key(m, 0, 0));
        list.Add(Key(vk, 0, 0)); list.Add(Key(vk, 0, 2));
        for (int i = mods.Length - 1; i >= 0; i--) list.Add(Key(mods[i], 0, 2));
        SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
    }
    public static void CtrlAltF() {
        SendInput(6, new[] { Key(0x11, 0, 0), Key(0x12, 0, 0), Key(0x46, 0, 0), Key(0x46, 0, 2), Key(0x12, 0, 2), Key(0x11, 0, 2) }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        System.Threading.Thread.Sleep(80);
        SendInput(2, new[] { Mouse(2), Mouse(4) }, Marshal.SizeOf(typeof(INPUT)));
    }
    // The screen inside (x, y, w, h), layered windows included (CAPTUREBLT).
    public static Bitmap Grab(int x, int y, int w, int h) {
        if (x < 0) { w += x; x = 0; }
        if (y < 0) { h += y; y = 0; }
        w = Math.Max(1, Math.Min(w, ScreenW - x));
        h = Math.Max(1, Math.Min(h, ScreenH - y));
        var bmp = new Bitmap(w, h, PixelFormat.Format24bppRgb);
        using (var g = Graphics.FromImage(bmp)) {
            IntPtr dst = g.GetHdc();
            IntPtr src = GetDC(IntPtr.Zero);
            BitBlt(dst, 0, 0, w, h, src, x, y, 0x00CC0020 | 0x40000000);
            ReleaseDC(IntPtr.Zero, src);
            g.ReleaseHdc(dst);
        }
        return bmp;
    }
    public static void Save(Bitmap b, string path) { b.Save(path, ImageFormat.Png); }
    static byte[] Bytes(Bitmap b, out int stride) {
        var d = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format24bppRgb);
        stride = d.Stride;
        var buf = new byte[d.Stride * b.Height];
        Marshal.Copy(d.Scan0, buf, 0, buf.Length);
        b.UnlockBits(d);
        return buf;
    }
    // Mean luminance and its standard deviation (0..255).
    public static double[] Stats(Bitmap b) {
        int stride; var buf = Bytes(b, out stride);
        double sum = 0, sq = 0; long n = 0;
        for (int y = 0; y < b.Height; y++) for (int x = 0; x < b.Width; x++) {
            int i = y * stride + x * 3;
            double l = 0.114 * buf[i] + 0.587 * buf[i + 1] + 0.299 * buf[i + 2];
            sum += l; sq += l * l; n++;
        }
        double m = sum / n;
        return new[] { m, Math.Sqrt(Math.Max(0, sq / n - m * m)) };
    }
    // Mean absolute difference per channel of two same-size images.
    public static double Diff(Bitmap a, Bitmap b) {
        if (a.Width != b.Width || a.Height != b.Height) return 255;
        int sa, sb; var x = Bytes(a, out sa); var y = Bytes(b, out sb);
        double sum = 0; long n = 0;
        for (int r = 0; r < a.Height; r++) for (int c = 0; c < a.Width * 3; c++) { sum += Math.Abs(x[r * sa + c] - y[r * sb + c]); n++; }
        return sum / n;
    }
}
'@

Write-Host ("screen {0}x{1}, Windows {2}" -f [Q]::ScreenW, [Q]::ScreenH, [Environment]::OSVersion.Version)
(Get-CimInstance Win32_OperatingSystem).Caption | Write-Host

# Sample files whose names the queries also find.
$docs = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'quick-shots'
New-Item -ItemType Directory -Force $docs | Out-Null
foreach ($f in @('مشكلة الطابعة.txt', 'حل مشكلة الشبكة.docx', 'تقرير مالي 2026.xlsx', 'notepad tips.txt', 'Network problem notes.txt', 'youtube links.txt')) {
    Set-Content -LiteralPath (Join-Path $docs $f) -Value 'x' -Encoding utf8
}

$script:mode = 'fade'
$script:altSpace = 0
function Seed-Settings {
    New-Item -ItemType Directory -Force $data | Out-Null
    $now = [int][double]::Parse((Get-Date -UFormat %s))
    $text = "floating_asked=1`nfloating_mode=$($script:mode)`nhotkey_opens=quick`ntray_icon=1`nalt_space=$($script:altSpace)`nupdate_check=$now`n"
    [IO.File]::WriteAllText((Join-Path $data 'settings.txt'), $text, (New-Object Text.UTF8Encoding $false))
}
function Set-Lang([string]$lang) {
    New-Item -Force 'HKCU:\Software\BahthSari' | Out-Null
    Set-ItemProperty 'HKCU:\Software\BahthSari' -Name Language -Value $lang
}
function Set-Theme([bool]$light) {
    $k = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
    New-Item -Force $k -ErrorAction SilentlyContinue | Out-Null
    $v = [int]$light
    Set-ItemProperty $k -Name AppsUseLightTheme -Value $v -Type DWord
    Set-ItemProperty $k -Name SystemUsesLightTheme -Value $v -Type DWord
    [Q]::BroadcastTheme()
    Start-Sleep -Seconds 2
}
function Stop-All { Get-Process bahth -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 800 }
function Check-Alive([string]$where) {
    if ($script:proc -and $script:proc.HasExited) { Fail "bahth.exe exited during '$where' (code $($script:proc.ExitCode))"; return $false }
    if (Test-Path $crash) { Fail "crash.txt written during '$where'"; return $false }
    return $true
}

# Saves the screen area, scaled down when it is large; returns its stats.
function Shot([string]$name, [int]$x, [int]$y, [int]$w, [int]$h) {
    $b = [Q]::Grab($x, $y, $w, $h)
    [Q]::Save($b, (Join-Path $Out "$name.png"))
    $s = [Q]::Stats($b)
    $b.Dispose()
    Write-Host ("  shot {0}: {1}x{2} at {3},{4} mean {5:N0} sd {6:N1}" -f $name, $w, $h, $x, $y, $s[0], $s[1])
    return $s
}
function Shot-Window([string]$name, [IntPtr]$h, [int]$pad = 16) {
    $r = [Q]::Rect($h)
    return Shot $name ($r.L - $pad) ($r.T - $pad) ($r.R - $r.L + 2 * $pad) ($r.B - $r.T + 2 * $pad)
}

function Start-App([string]$exePath) {
    Stop-All
    Seed-Settings
    $p = Start-Process -FilePath $exePath -PassThru
    $script:proc = $p
    $deadline = (Get-Date).AddSeconds(60)
    $h = [IntPtr]::Zero
    while ((Get-Date) -lt $deadline -and $h -eq [IntPtr]::Zero -and -not $p.HasExited) {
        Start-Sleep -Milliseconds 200
        $h = [Q]::MainWindow($p.Id)
    }
    if ($h -eq [IntPtr]::Zero) { throw 'the main window did not appear' }
    $script:main = $h
    return $p
}

# ---------------------------------------------------------------- indexing
Write-Host 'Indexing…'
Set-Lang 'en'
Set-Theme $false
Remove-Item -Force -ErrorAction SilentlyContinue $logFile, $crash
$p = Start-App $Exe
$started = Get-Date
while (((Get-Date) - $started).TotalMinutes -lt 30 -and -not (Test-Path $index) -and -not $p.HasExited) { Start-Sleep -Seconds 5 }
if (-not (Test-Path $index)) { throw 'the index was not saved in time' }
Write-Host ("index saved after {0:N0} s" -f ((Get-Date) - $started).TotalSeconds)
Start-Sleep -Seconds 20
Check-Alive 'indexing' | Out-Null

# Windows 11 puts a new program's tray icon in the overflow; show it on the
# taskbar like a user who dragged it there.
$promoted = 0
Get-ChildItem 'HKCU:\Control Panel\NotifyIconSettings' -ErrorAction SilentlyContinue | ForEach-Object {
    $path = (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).ExecutablePath
    if ($path -and $path -like '*bahth.exe') { Set-ItemProperty $_.PSPath -Name IsPromoted -Value 1 -Type DWord; $promoted++ }
}
Write-Host "tray icon promoted: $promoted entries"

# ---------------------------------------------------------------- the quick search
function Popup { return [Q]::Find($script:proc.Id, 'InstaFindQuickSearch') }
function Popup-Height { $h = Popup; if ($h -eq [IntPtr]::Zero) { return 0 }; $r = [Q]::Rect($h); return $r.B - $r.T }
function Popup-Visible { $h = Popup; return ($h -ne [IntPtr]::Zero) -and [Q]::IsWindowVisible($h) }
function Far-Cursor { [Q]::SetCursorPos(40, 40) | Out-Null }

function Open-Popup([string]$where) {
    for ($i = 0; $i -lt 3; $i++) {
        [Q]::CtrlAltF()
        for ($t = 0; $t -lt 20 -and -not (Popup-Visible); $t++) { Start-Sleep -Milliseconds 100 }
        if (Popup-Visible) { Start-Sleep -Milliseconds 400; return $true }
    }
    Fail "the popup did not open with Ctrl+Alt+F ($where)"
    return $false
}
function Type-Query([string]$q) {
    $edit = [Q]::Child((Popup), 'Edit')
    if ($edit -eq [IntPtr]::Zero) { Fail 'no search box in the popup'; return }
    [Q]::SetText($edit, '')
    Start-Sleep -Milliseconds 150
    foreach ($c in $q.ToCharArray()) { [Q]::Char($c); Start-Sleep -Milliseconds 40 }
    Start-Sleep -Milliseconds 1500   # file search, catalog
}
function Close-Popup([string]$where) {
    [Q]::VKey(0x1B)
    for ($t = 0; $t -lt 20 -and (Popup-Visible); $t++) { Start-Sleep -Milliseconds 100 }
    if (Popup-Visible) { Fail "Esc did not close the popup ($where)" }
}

$script:heights = @{}
function Session([string]$lang, [bool]$light, [string]$tag, [object[]]$queries, [bool]$full) {
    Write-Host "== session $tag"
    Set-Lang $lang
    Set-Theme $light
    $p = Start-App $Exe
    Start-Sleep -Seconds 3
    [Q]::ShowWindow($script:main, 6) | Out-Null   # minimise: the desktop shows behind
    Far-Cursor
    $box = [IntPtr]::Zero
    for ($t = 0; $t -lt 40 -and $box -eq [IntPtr]::Zero; $t++) { Start-Sleep -Milliseconds 250; $box = [Q]::Find($p.Id, 'InstaFindFloatingBox') }
    $sw = [Q]::ScreenW; $sh = [Q]::ScreenH
    if ($box -eq [IntPtr]::Zero) {
        Fail "no floating box ($tag)"
    } else {
        Start-Sleep -Seconds 4   # the wake-up glow fades out
        $r = [Q]::Rect($box)
        Note ("[$tag] floating box at {0},{1} size {2}x{3} on a {4}x{5} screen" -f $r.L, $r.T, ($r.R - $r.L), ($r.B - $r.T), $sw, $sh)
        if ($r.R -gt $sw -or $r.B -gt $sh -or $r.L -lt 0 -or $r.T -lt 0) { Fail "floating box off screen ($tag)" }
        $cx = 760; $cy = 140
        $idle = [Q]::Grab($sw - $cx, $sh - $cy, $cx, $cy)
        [Q]::Save($idle, (Join-Path $Out "floating-idle-$tag.png"))
        [Q]::SetCursorPos([int](($r.L + $r.R) / 2), [int](($r.T + $r.B) / 2)) | Out-Null
        Start-Sleep -Milliseconds 1500
        $hover = [Q]::Grab($sw - $cx, $sh - $cy, $cx, $cy)
        [Q]::Save($hover, (Join-Path $Out "floating-hover-$tag.png"))
        $d = [Q]::Diff($idle, $hover)
        Note ("[$tag] floating box idle vs hovered: mean difference {0:N2}" -f $d)
        if ($d -lt 0.05) { Fail "the floating box does not fade in on hover ($tag)" }
        $idle.Dispose(); $hover.Dispose()
        if ($full) {
            # A click on the box opens the popup right above it.
            [Q]::Click([int](($r.L + $r.R) / 2), [int](($r.T + $r.B) / 2))
            for ($t = 0; $t -lt 20 -and -not (Popup-Visible); $t++) { Start-Sleep -Milliseconds 100 }
            if (Popup-Visible) {
                Start-Sleep -Milliseconds 500
                $pr = [Q]::Rect((Popup))
                Note ("[$tag] popup from the box at {0},{1} size {2}x{3}" -f $pr.L, $pr.T, ($pr.R - $pr.L), ($pr.B - $pr.T))
                $top = [Math]::Min($pr.T, $r.T) - 20
                Shot "box-click-$tag" ($sw - 760) $top 760 ($sh - $top) | Out-Null
                Close-Popup "box click $tag"
            } else { Fail "a click on the floating box did not open the popup ($tag)" }
        }
        Far-Cursor
    }
    Check-Alive "floating $tag" | Out-Null

    if (-not (Open-Popup $tag)) { return }
    $empty = Popup-Height
    $s = Shot-Window "popup-empty-$tag" (Popup)
    Note ("[$tag] popup empty: height {0}, mean {1:N0}" -f $empty, $s[0])
    $script:heights["$tag-empty"] = $empty
    $script:means["$tag"] = $s[0]
    foreach ($q in $queries) {
        Type-Query $q.Text
        $h = Popup-Height
        $s = Shot-Window "q-$($q.Name)-$tag" (Popup)
        Note ("[$tag] '{0}': height {1} (empty {2}), sd {3:N1}" -f $q.Text, $h, $empty, $s[1])
        if (-not (Popup-Visible)) { Fail "the popup closed while typing '$($q.Text)' ($tag)"; if (-not (Open-Popup $tag)) { return } }
        elseif ($q.Expect -and $h -le $empty) { Fail "no results for '$($q.Text)' ($tag): the popup did not grow" }
        Check-Alive "query $($q.Text) $tag" | Out-Null
    }
    if ($full) {
        # The action menu of the first row.
        Type-Query $queries[0].Text
        [Q]::VKey(0x09)
        $menu = [IntPtr]::Zero
        for ($t = 0; $t -lt 20 -and $menu -eq [IntPtr]::Zero; $t++) { Start-Sleep -Milliseconds 100; $menu = [Q]::FindVisible('#32768') }
        if ($menu -eq [IntPtr]::Zero) { Fail "Tab did not open the action menu ($tag)" } else {
            Start-Sleep -Milliseconds 400
            $pr = [Q]::Rect((Popup)); $mr = [Q]::Rect($menu)
            $l = [Math]::Min($pr.L, $mr.L) - 16; $t2 = [Math]::Min($pr.T, $mr.T) - 16
            $rr = [Math]::Max($pr.R, $mr.R) + 16; $b2 = [Math]::Max($pr.B, $mr.B) + 16
            Shot "tab-menu-$tag" $l $t2 ($rr - $l) ($b2 - $t2) | Out-Null
            [Q]::VKey(0x1B)   # closes the menu only
            Start-Sleep -Milliseconds 400
            if (-not (Popup-Visible)) { Fail "closing the action menu closed the popup ($tag)" }
        }
    }
    Close-Popup $tag
    Check-Alive "close $tag" | Out-Null
    if ($full) {
        Far-Cursor
        Start-Sleep -Seconds 2
        Shot "tray-$tag" ($sw - 420) ($sh - 60) 420 60 | Out-Null
    }
    if (Test-Path $logFile) { Copy-Item $logFile (Join-Path $Out "log-$tag.txt") -Force }
}

$script:means = @{}
$common = @(
    @{ Name = 'wifi';     Text = 'واي فاي';        Expect = $true },
    @{ Name = 'calc';     Text = '250*12';          Expect = $true },
    @{ Name = 'units';    Text = '5 kg to lb';      Expect = $true },
    @{ Name = 'hijri';    Text = 'هجري';            Expect = $true },
    @{ Name = 'youtube';  Text = 'يوتيوب قرآن';     Expect = $true },
    @{ Name = 'shutdown'; Text = 'اطفاء';           Expect = $true },   # never Enter
    @{ Name = 'problem';  Text = 'مشكلة';           Expect = $true },
    @{ Name = 'nonsense'; Text = 'qzxwvkjh';        Expect = $false }
)
$ar = @(@{ Name = 'app'; Text = 'المفكرة'; Expect = $true }, @{ Name = 'chrome'; Text = 'كروم'; Expect = $true }) + $common
$en = @(@{ Name = 'app'; Text = 'notepad'; Expect = $true }, @{ Name = 'paint'; Text = 'paint'; Expect = $true }) + $common
$light = @(@{ Name = 'app'; Text = 'calculator'; Expect = $true }, @{ Name = 'calc'; Text = '250*12'; Expect = $true }, @{ Name = 'problem'; Text = 'مشكلة'; Expect = $true })

# What the desktop looks like when something did not react.
function Diag([string]$where) {
    $c = New-Object Q+POINT
    [Q]::GetCursorPos([ref]$c) | Out-Null
    $fg = [Q]::GetForegroundWindow()
    $resp = if ($script:proc -and -not $script:proc.HasExited) { $script:proc.Refresh(); $script:proc.Responding } else { 'exited' }
    Write-Host ("  diag {0}: cursor {1},{2}; foreground '{3}' '{4}'; bahth responding: {5}" -f $where, $c.X, $c.Y, [Q]::Class($fg), [Q]::Title($fg), $resp)
    Shot "diag-$where" 0 0 ([Q]::ScreenW) ([Q]::ScreenH) | Out-Null
}

# A session that fails is run once more: the runner's desktop sometimes
# stops taking input for a moment. Both attempts are reported.
function Session-Twice([string]$lang, [bool]$light, [string]$tag, [object[]]$queries, [bool]$full) {
    $before = $script:failed.Count
    try { Session $lang $light $tag $queries $full } catch { Fail "session $tag failed: $($_.Exception.Message) at $($_.InvocationInfo.PositionMessage)" }
    if ($script:failed.Count -eq $before) { return }
    Diag $tag
    $first = @($script:failed[$before..($script:failed.Count - 1)])
    $script:failed = @(if ($before -gt 0) { $script:failed[0..($before - 1)] })
    Note "[$tag] first attempt failed ($($first -join '; ')); running the session again"
    try { Session $lang $light $tag $queries $full } catch { Fail "session $tag failed: $($_.Exception.Message) at $($_.InvocationInfo.PositionMessage)" }
    if ($script:failed.Count -gt $before) { Diag "$tag-2" }
}
Session-Twice 'ar' $false 'ar' $ar $true
Session-Twice 'en' $false 'en' $en $true
Session-Twice 'ar' $true 'ar-light' $light $false
if ($script:means['ar-light'] -and $script:means['ar'] -and $script:means['ar-light'] -le $script:means['ar']) {
    Write-Host "::warning::the light popup is not lighter than the dark one"
}
Set-Theme $false

# ---------------------------------------------------------------- extras
function Log-Has([string]$pattern) { return (Test-Path $logFile) -and ([IO.File]::ReadAllText($logFile) -match $pattern) }
try {
    Write-Host '== extras'
    Set-Lang 'en'
    $script:altSpace = 1
    $p = Start-App $Exe
    Start-Sleep -Seconds 4
    [Q]::ShowWindow($script:main, 6) | Out-Null
    Far-Cursor
    Start-Sleep -Seconds 1
    # Alt+Space opens the quick search when it is turned on.
    [Q]::Combo(@(0x12), 0x20)
    for ($t = 0; $t -lt 20 -and -not (Popup-Visible); $t++) { Start-Sleep -Milliseconds 100 }
    if (Popup-Visible) { Note '[extras] Alt+Space opened the quick search'; Close-Popup 'alt space' } else { Fail 'Alt+Space did not open the quick search (alt_space=1)' }
    if (Log-Has 'hotkey Alt\+Space is taken') { Fail 'Alt+Space could not be registered' }

    # Ctrl+Alt+G with an Open dialog of Notepad in front.
    $np = Start-Process notepad -PassThru
    Start-Sleep -Seconds 3
    $npw = [Q]::MainWindow($np.Id)
    if ($npw -ne [IntPtr]::Zero) { [Q]::SetForegroundWindow($npw) | Out-Null }
    Start-Sleep -Milliseconds 500
    [Q]::Combo(@(0x11), 0x4F)   # Ctrl+O
    $dlg = [IntPtr]::Zero
    for ($t = 0; $t -lt 30 -and $dlg -eq [IntPtr]::Zero; $t++) { Start-Sleep -Milliseconds 200; $dlg = [Q]::FindVisible('#32770') }
    if ($dlg -eq [IntPtr]::Zero) { Write-Host '::warning::no Open dialog in Notepad; Ctrl+Alt+G tested without one' }
    for ($try = 0; $try -lt 3; $try++) {
        if ($dlg -ne [IntPtr]::Zero) {
            # A click on its title bar puts the dialog in front (SetForegroundWindow
            # from this script is not always allowed to).
            $dr = [Q]::Rect($dlg)
            [Q]::Click([int](($dr.L + $dr.R) / 2), $dr.T + 12)
            Start-Sleep -Milliseconds 600
            Write-Host "  in front before Ctrl+Alt+G: '$([Q]::Class([Q]::GetForegroundWindow()))'"
        }
        [Q]::Combo(@(0x11, 0x12), 0x47)   # Ctrl+Alt+G
        Start-Sleep -Seconds 2
        if ($dlg -eq [IntPtr]::Zero -or (Log-Has 'quick switch: hotkey, file dialog found')) { break }
        [Q]::ShowWindow($script:main, 6) | Out-Null
    }
    if (-not (Log-Has 'quick switch: hotkey')) { Fail 'Ctrl+Alt+G did nothing (no "quick switch: hotkey" in log.txt)' }
    elseif ($dlg -ne [IntPtr]::Zero -and -not (Log-Has 'quick switch: hotkey, file dialog found')) { Fail 'Ctrl+Alt+G did not see the Open dialog in front' }
    else { Note "[extras] Ctrl+Alt+G: $(([regex]::Matches([IO.File]::ReadAllText($logFile), 'quick switch: hotkey[^\r\n]*') | Select-Object -Last 1).Value)" }
    Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    [Q]::ShowWindow($script:main, 6) | Out-Null

    # Classic tools from the settings results open (Enter; these are not system commands).
    foreach ($tool in @(@{ Q = 'device manager'; Proc = 'mmc'; Name = 'devmgmt' }, @{ Q = 'devices and printers'; Proc = $null; Name = 'printers' })) {
        if (-not (Open-Popup "tool $($tool.Name)")) { continue }
        Type-Query $tool.Q
        Shot-Window "tool-$($tool.Name)" (Popup) | Out-Null
        $before = @(Get-Process | ForEach-Object Id)
        [Q]::VKey(0x0D)
        Start-Sleep -Seconds 5
        $new = Get-Process | Where-Object { $before -notcontains $_.Id }
        Note ("[extras] Enter on '{0}' started: {1}" -f $tool.Q, (($new | ForEach-Object { $_.ProcessName } | Sort-Object -Unique) -join ', '))
        if ($tool.Proc -and -not ($new | Where-Object ProcessName -eq $tool.Proc)) { Fail "Enter on '$($tool.Q)' did not start $($tool.Proc).exe" }
        if (Log-Has 'Could not open|system command .* failed') { Fail "opening '$($tool.Q)' failed (log)" }
        Shot "tool-$($tool.Name)-opened" 0 0 ([Q]::ScreenW) ([Q]::ScreenH) | Out-Null
        if (Popup-Visible) { Close-Popup "tool $($tool.Name)" }
        $new | Where-Object { $_.ProcessName -in @('mmc', 'control', 'SystemSettings') } | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 1
    }
    Check-Alive 'extras' | Out-Null

    # floating_mode=hidden: the invisible box lets clicks through.
    $script:altSpace = 0
    $script:mode = 'hidden'
    $p = Start-App $Exe
    Start-Sleep -Seconds 3
    [Q]::ShowWindow($script:main, 6) | Out-Null
    Far-Cursor
    Start-Sleep -Seconds 4
    $box = [Q]::Find($p.Id, 'InstaFindFloatingBox')
    if ($box -eq [IntPtr]::Zero) { Fail 'no floating box in hidden mode' } else {
        $r = [Q]::Rect($box)
        $cls = [Q]::ClassAt([int](($r.L + $r.R) / 2), [int](($r.T + $r.B) / 2))
        Note "[extras] hidden mode, cursor away: a click on the box's place reaches '$cls'"
        if ($cls -eq 'InstaFindFloatingBox') { Fail 'in hidden mode the invisible box catches clicks' }
        Shot 'floating-hidden-en' ([Q]::ScreenW - 760) ([Q]::ScreenH - 140) 760 140 | Out-Null
    }
    $script:mode = 'fade'
    Check-Alive 'hidden mode' | Out-Null
} catch {
    Fail "extras failed: $($_.Exception.Message) at $($_.InvocationInfo.PositionMessage)"
}

# The catalog line in the log.
$logText = if (Test-Path $logFile) { [IO.File]::ReadAllText($logFile) } else { '' }
$m = [regex]::Matches($logText, 'launcher catalog: (\d+) apps, (\d+) settings pages in (\d+) ms')
if ($m.Count -eq 0) { Fail 'no "launcher catalog" line in log.txt' } else {
    $last = $m[$m.Count - 1]
    Note "catalog: $($last.Value)"
    if ([int]$last.Groups[1].Value -lt 5) { Fail "the catalog found only $($last.Groups[1].Value) apps" }
    if ([int]$last.Groups[2].Value -lt 10) { Fail "the catalog has only $($last.Groups[2].Value) settings pages" }
}
foreach ($x in [regex]::Matches($logText, '(quick search (window built|shown) in \d+ ms|floating box built in \d+ ms)')) { Write-Host "  log: $($x.Value)" }
if (Test-Path $crash) { Copy-Item $crash (Join-Path $Out 'crash.txt'); Fail 'crash.txt was written' }
Stop-All

# ---------------------------------------------------------------- speed against 0.6.1
function Median([double[]]$v) { $s = $v | Sort-Object; return $s[[int][Math]::Floor($s.Count / 2)] }
# Start-Process -Wait polls about once a second, so the process is run
# and waited for directly.
function Cli-Time([string]$exePath, [string]$q) {
    $si = New-Object Diagnostics.ProcessStartInfo
    $si.FileName = $exePath
    $si.Arguments = "-s `"$q`" --all --scope C: --count"
    $si.UseShellExecute = $false
    $si.RedirectStandardOutput = $true
    $si.StandardOutputEncoding = [Text.Encoding]::UTF8
    $si.CreateNoWindow = $true
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $pr = [Diagnostics.Process]::Start($si)
    $t = $pr.StandardOutput.ReadToEnd()
    $pr.WaitForExit()
    $sw.Stop()
    $t = $t.Trim()
    $n = if ($t -match '(\d+)') { [int]$Matches[1] } else { -1 }
    return @($sw.Elapsed.TotalMilliseconds, $n)
}
function Startup-Time([string]$exePath) {
    $before = if (Test-Path $logFile) { [regex]::Matches([IO.File]::ReadAllText($logFile), 'startup ms').Count } else { 0 }
    Stop-All
    Seed-Settings
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $exePath -PassThru
    $shown = $null
    while ($sw.Elapsed.TotalSeconds -lt 30 -and $null -eq $shown) {
        if ([Q]::MainWindow($p.Id) -ne [IntPtr]::Zero) { $shown = $sw.Elapsed.TotalMilliseconds } else { Start-Sleep -Milliseconds 10 }
    }
    $phases = $null
    for ($t = 0; $t -lt 100 -and $null -eq $phases; $t++) {
        Start-Sleep -Milliseconds 100
        if (Test-Path $logFile) {
            $all = [regex]::Matches([IO.File]::ReadAllText($logFile), 'startup ms \(v[^)]*\): ([^\r\n]*)')
            if ($all.Count -gt $before) { $phases = $all[$all.Count - 1].Groups[1].Value }
        }
    }
    Start-Sleep -Seconds 2
    Stop-All
    $get = { param($name) if ($phases -match "(^| )$name=(\d+)") { [double]$Matches[2] } else { [double]::NaN } }
    return @{ Shown = $shown; Built = (& $get 'built'); Layout = (& $get 'layout'); FirstTick = (& $get 'first-tick'); Phases = $phases }
}

$md = @()
if ($OldExe -and (Test-Path $OldExe)) {
    $OldExe = (Resolve-Path $OldExe).Path
    Set-Lang 'en'
    $queries = @('report', 'notepad', 'احمد', 'تقرير مالي', 'مشكلة', 'ext:dll', 'system32 config')
    $rows = @()
    foreach ($q in $queries) {
        Cli-Time $OldExe $q | Out-Null; Cli-Time $Exe $q | Out-Null   # warm-up
        $a = @(); $b = @(); $na = 0; $nb = 0
        for ($i = 0; $i -lt 5; $i++) {
            $r = Cli-Time $OldExe $q; $a += $r[0]; $na = $r[1]
            $r = Cli-Time $Exe $q; $b += $r[0]; $nb = $r[1]
        }
        $rows += [pscustomobject]@{ Query = $q; Old = (Median $a); New = (Median $b); OldN = $na; NewN = $nb }
        Write-Host ("  {0}: 0.6.1 {1:N0} ms ({2}), new {3:N0} ms ({4})" -f $q, (Median $a), $na, (Median $b), $nb)
    }
    $st = @{ Old = @(); New = @() }
    for ($i = 0; $i -lt 5; $i++) {
        $st.Old += Startup-Time $OldExe
        $st.New += Startup-Time $Exe
    }
    $os = (Get-CimInstance Win32_OperatingSystem).Caption
    $cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
    $vo = Join-Path $env:RUNNER_TEMP 'qs-version.txt'
    Start-Process -FilePath $Exe -ArgumentList '--version' -NoNewWindow -Wait -RedirectStandardOutput $vo | Out-Null
    $ver = [IO.File]::ReadAllText($vo).Trim()
    $md += "# Speed: this build against 0.6.1"
    $md += ''
    $md += "Measured by ``ci/quick-shots.ps1`` on a GitHub Actions runner: $os, $cpu, $(Get-Date -Format 'yyyy-MM-dd'). This build: $ver. Both exes search the same saved index ($([Math]::Round((Get-Item $index).Length / 1MB)) MB); medians of 5 runs, taken alternately."
    $md += ''
    $md += '## Command-line name search (`bahth.exe -s "…" --all --scope C: --count`, loads the index each run)'
    $md += ''
    $md += '| Query | 0.6.1 (ms) | this build (ms) | change | found (0.6.1 / this) |'
    $md += '|---|---:|---:|---:|---:|'
    $slow = @()
    foreach ($r in $rows) {
        $ch = ($r.New - $r.Old) / $r.Old * 100
        $md += ("| ``{0}`` | {1:N0} | {2:N0} | {3:+0;-0}% | {4} / {5} |" -f $r.Query, $r.Old, $r.New, $ch, $r.OldN, $r.NewN)
        if ($r.OldN -ne $r.NewN) { Fail "'$($r.Query)' finds $($r.OldN) with 0.6.1 but $($r.NewN) now" }
    }
    $totOld = ($rows | Measure-Object Old -Sum).Sum; $totNew = ($rows | Measure-Object New -Sum).Sum
    $md += ("| **all** | **{0:N0}** | **{1:N0}** | **{2:+0;-0}%** | |" -f $totOld, $totNew, (($totNew - $totOld) / $totOld * 100))
    if ($totNew -gt $totOld * 1.05) { Fail ("the command line is slower than 0.6.1: {0:N0} ms against {1:N0} ms" -f $totNew, $totOld) }
    $md += ''
    $md += '## Start-up of the main window (index already saved)'
    $md += ''
    $md += 'Shown = from `Start-Process` until the window is visible (measured outside). The phases are the program''s own `startup ms` line in log.txt, from process creation: `built` = window and controls created, `layout` = laid out, `first-tick` = first timer tick after the first paint.'
    $md += ''
    $md += '| Phase (median ms) | 0.6.1 | this build | change |'
    $md += '|---|---:|---:|---:|'
    foreach ($k in @('Shown', 'Built', 'Layout', 'FirstTick')) {
        $o = Median ($st.Old | ForEach-Object { $_[$k] }); $n = Median ($st.New | ForEach-Object { $_[$k] })
        $ch = if ($o -gt 0) { "{0:+0;-0}%" -f (($n - $o) / $o * 100) } else { '' }
        $md += ("| {0} | {1:N0} | {2:N0} | {3} |" -f $k, $o, $n, $ch)
        if ($k -eq 'FirstTick' -and $n -gt $o * 1.05 + 20) { Fail ("start-up is slower than 0.6.1: first tick {0:N0} ms against {1:N0} ms" -f $n, $o) }
    }
    $md += ''
    $md += 'Raw phases (last run of each):'
    $md += ''
    $md += "- 0.6.1: ``$($st.Old[-1].Phases)``"
    $md += "- this build: ``$($st.New[-1].Phases)``"
    $quick = [regex]::Matches($logText, 'quick search shown in (\d+) ms') | ForEach-Object { [double]$_.Groups[1].Value }
    if ($quick) {
        $md += ''
        $md += ("Quick search popup (this build only): opened in a median of {0:N0} ms over {1} openings (``quick search shown in`` in log.txt)." -f (Median $quick), @($quick).Count)
    }
    if (Test-Path $crash) { Fail 'crash.txt was written during the speed runs' }
} else {
    Write-Host '::warning::no 0.6.1 exe: skipped the speed comparison'
}

$md += ''
$md += '## Checks'
$md += ''
if (Test-Path (Join-Path $Out 'checks.txt')) { $md += (Get-Content (Join-Path $Out 'checks.txt') -Encoding utf8 | ForEach-Object { "- $_" }) }
if ($script:failed.Count) { $md += ''; $md += '**Failed:**'; $md += ''; $md += ($script:failed | ForEach-Object { "- $_" }) } else { $md += ''; $md += 'All checks passed.' }
[IO.File]::WriteAllText((Join-Path $Out 'speed.md'), ($md -join "`n") + "`n", (New-Object Text.UTF8Encoding $false))
Get-Content (Join-Path $Out 'speed.md') -Encoding utf8 | Write-Host
if ($env:GITHUB_STEP_SUMMARY) { Get-Content (Join-Path $Out 'speed.md') -Encoding utf8 | Add-Content $env:GITHUB_STEP_SUMMARY -Encoding utf8 }
if (Test-Path $logFile) { Copy-Item $logFile (Join-Path $Out 'log.txt') -Force }
Get-ChildItem $Out | Format-Table Name, Length | Out-String | Write-Host
if ($script:failed.Count) { Write-Host "$($script:failed.Count) check(s) failed"; exit 1 }
Write-Host 'All quick-search checks passed.'
exit 0
