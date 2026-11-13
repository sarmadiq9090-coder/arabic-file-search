# Microsoft Store screenshots on a real Windows machine (store.yml): sets the
# screen to 1920x1080 when the runner allows it, seeds sample files with
# Arabic and English names in C:\Users\Public\Demo (so no runner user folder
# shows in a path), lets the program index, and captures the full screen in
# Arabic and in English, in the new look (WebView2: the only one since 1.0):
#   1 name search («تقرير» / "report")   2 search inside documents
#   3 the Ctrl+Alt+F quick search        4 the duplicate finder
# Everything goes to $Out with resolution.txt (the resolution really used).
# Fails on a crash or a missing window; a scene that shows no results is a
# warning, so the other shots still come out.
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out,
    [int]$Width = 1920,
    [int]$Height = 1080
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$index = Join-Path $data 'index.bin'
$crash = Join-Path $data 'crash.txt'
$script:warnings = @()
function Warn([string]$msg) { Write-Host "::warning::$msg"; $script:warnings += $msg }

Add-Type -AssemblyName System.Drawing, System.Windows.Forms, UIAutomationClient, UIAutomationTypes
Add-Type -ReferencedAssemblies System.Drawing @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
public static class S {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit, Size = 40)] struct INPUT {
        [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; [FieldOffset(8)] public MOUSEINPUT mi;
    }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] struct DEVMODE {
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
        public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra;
        public int dmFields, dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
        public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
        public short dmLogPixels;
        public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
        public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
    }
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, string l);
    [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint item, StringBuilder sb, int n, uint flags);
    [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint item, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool EnumDisplaySettings(string dev, int mode, ref DEVMODE dm);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int ChangeDisplaySettings(ref DEVMODE dm, int flags);

    static INPUT Key(ushort vk, ushort scan, uint flags) {
        var i = new INPUT(); i.type = 1; i.ki.vk = vk; i.ki.scan = scan; i.ki.flags = flags; return i;
    }
    static INPUT Mouse(uint flags) { var i = new INPUT(); i.type = 0; i.mi.flags = flags; return i; }

    // Every mode the display offers, "WxH", and a switch to one of them.
    public static string[] Modes() {
        var seen = new List<string>();
        var dm = new DEVMODE(); dm.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
        for (int i = 0; EnumDisplaySettings(null, i, ref dm); i++) {
            string m = dm.dmPelsWidth + "x" + dm.dmPelsHeight;
            if (!seen.Contains(m)) seen.Add(m);
        }
        return seen.ToArray();
    }
    public static int SetResolution(int w, int h) {
        var dm = new DEVMODE(); dm.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
        if (!EnumDisplaySettings(null, -1 /* ENUM_CURRENT_SETTINGS */, ref dm)) return -100;
        dm.dmPelsWidth = w; dm.dmPelsHeight = h;
        dm.dmFields = 0x80000 | 0x100000;   // DM_PELSWIDTH | DM_PELSHEIGHT
        return ChangeDisplaySettings(ref dm, 0);   // 0 = DISP_CHANGE_SUCCESSFUL
    }

    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
    public static string Text(IntPtr h) { var s = new StringBuilder(512); GetWindowText(h, s, 512); return s.ToString(); }
    public static string ParentClass(IntPtr h) { return Class(GetParent(h)); }
    public static List<IntPtr> TopWindows(int pid) {
        var list = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if ((pid == 0 || p == pid) && IsWindowVisible(h)) list.Add(h);
            return true;
        }, IntPtr.Zero);
        return list;
    }
    // The process's largest visible top-level window.
    public static IntPtr MainWindow(int pid) {
        IntPtr best = IntPtr.Zero; long area = 0;
        foreach (var h in TopWindows(pid)) {
            RECT r; GetWindowRect(h, out r);
            long a = (long)(r.R - r.L) * (r.B - r.T);
            if (a > area) { area = a; best = h; }
        }
        return best;
    }
    public static List<IntPtr> Children(IntPtr parent) {
        var list = new List<IntPtr>();
        EnumChildWindows(parent, (h, l) => { list.Add(h); return true; }, IntPtr.Zero);
        return list;
    }
    public static void Press(IntPtr h) { SendMessage(h, 0x00F5 /* BM_CLICK */, IntPtr.Zero, IntPtr.Zero); }
    public static void ClickAt(int x, int y) {
        SetCursorPos(x, y);
        System.Threading.Thread.Sleep(80);
        SendInput(2, new[] { Mouse(2), Mouse(4) }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Click(IntPtr h) {
        RECT r; GetWindowRect(h, out r);
        ClickAt((r.L + r.R) / 2, (r.T + r.B) / 2);
    }
    // The mouse out of the way of the shot, in the bottom corner.
    public static void Park(int w, int h) { SetCursorPos(w - 2, h - 60); }
    public static void SetText(IntPtr h, string t) { SendMessage(h, 0x000C, IntPtr.Zero, t); }
    public static bool Checked(IntPtr h) { return SendMessage(h, 0x00F0, IntPtr.Zero, IntPtr.Zero) == (IntPtr)1; }
    public static int ListCount(IntPtr h) { return (int)SendMessage(h, 0x1004 /* LVM_GETITEMCOUNT */, IntPtr.Zero, IntPtr.Zero); }
    // Types text as Unicode input, so Arabic needs no keyboard layout.
    public static void Type(string text) {
        foreach (char c in text) {
            SendInput(2, new[] { Key(0, c, 4), Key(0, c, 4 | 2) }, Marshal.SizeOf(typeof(INPUT)));
            System.Threading.Thread.Sleep(40);
        }
    }
    public static void Chord(params ushort[] vks) {
        var list = new List<INPUT>();
        foreach (var v in vks) list.Add(Key(v, 0, 0));
        for (int i = vks.Length - 1; i >= 0; i--) list.Add(Key(vks[i], 0, 2));
        SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
    }
    // The item of an open popup menu (#32768) whose text starts with one of
    // the given strings: { HMENU, position }, or null. nwg menus notify by
    // position (MNS_NOTIFYBYPOS), so WM_MENUCOMMAND(position, HMENU) posted
    // to the owner window selects it.
    public static long[] FindMenuItem(IntPtr menuWindow, string[] starts) {
        IntPtr m = SendMessage(menuWindow, 0x01E1 /* MN_GETHMENU */, IntPtr.Zero, IntPtr.Zero);
        int n = GetMenuItemCount(m);
        for (int i = 0; i < n; i++) {
            var sb = new StringBuilder(512);
            GetMenuString(m, (uint)i, sb, 512, 0x400 /* MF_BYPOSITION */);
            foreach (var s in starts) {
                if (s.Length > 0 && sb.ToString().StartsWith(s)) return new long[] { m.ToInt64(), i };
            }
        }
        return null;
    }
    public static string[] MenuTexts(IntPtr menuWindow) {
        IntPtr m = SendMessage(menuWindow, 0x01E1, IntPtr.Zero, IntPtr.Zero);
        var list = new List<string>();
        for (int i = 0; i < GetMenuItemCount(m); i++) {
            var sb = new StringBuilder(512);
            GetMenuString(m, (uint)i, sb, 512, 0x400);
            list.Add(sb.ToString());
        }
        return list.ToArray();
    }
    public static void Screen(string path, int w, int h) {
        using (var bmp = new Bitmap(w, h))
        using (var g = Graphics.FromImage(bmp)) {
            g.CopyFromScreen(0, 0, 0, 0, bmp.Size);
            bmp.Save(path, ImageFormat.Png);
        }
    }
}
'@
[void][S]::SetProcessDPIAware()
$VK = @{ CTRL = 0x11; ALT = 0x12; F = 0x46; ESC = 0x1B; ENTER = 0x0D; DOWN = 0x28; H = 0x48; COMMA = 0xBC }

# ---------------------------------------------------------------- screen size
Write-Host "display modes: $([S]::Modes() -join ', ')"
$set = [S]::SetResolution($Width, $Height)
Write-Host "ChangeDisplaySettings ${Width}x${Height} -> $set"
if ($set -ne 0 -and (Get-Command Set-DisplayResolution -ErrorAction SilentlyContinue)) {
    try { Set-DisplayResolution -Width $Width -Height $Height -Force; Write-Host 'Set-DisplayResolution done' } catch { Write-Host "Set-DisplayResolution: $_" }
}
Start-Sleep -Seconds 3
$screen = [Windows.Forms.Screen]::PrimaryScreen.Bounds
$W = $screen.Width; $H = $screen.Height
"${W}x${H}" | Set-Content (Join-Path $Out 'resolution.txt')
Write-Host "screen is ${W}x${H}"
if ($W -ne $Width -or $H -ne $Height) { Warn "the runner's screen is ${W}x${H}, not ${Width}x${Height}" }
if ($W -lt 1366 -or $H -lt 768) { Warn "${W}x${H} is below the Store's 1366x768 minimum" }

# The runner's own windows (its agent console) off the desktop.
function Clear-Desktop { try { (New-Object -ComObject Shell.Application).MinimizeAll() } catch { Write-Host "MinimizeAll: $_" }; Start-Sleep -Seconds 1 }
Clear-Desktop

# ---------------------------------------------------------------- sample files
# Realistic names in both languages; the .txt files carry the words the
# inside-documents scene finds; the backup folder holds exact copies for the
# duplicate finder.
$demo = 'C:\Users\Public\Demo'
Remove-Item $demo -Recurse -Force -ErrorAction SilentlyContinue
$files = [ordered]@{
    'العمل\تقرير المبيعات 2026.xlsx' = 'مبيعات الربع الثالث'
    'العمل\التقرير السنوي النهائي.pdf' = 'التقرير السنوي'
    'العمل\تقرير الاجتماع الأسبوعي.docx' = 'محضر الاجتماع الأسبوعي'
    'العمل\تقرير مالي - الربع الثالث.pdf' = 'التقرير المالي'
    'العمل\ميزانية المشروع.txt' = 'الميزانية التقديرية للمشروع الجديد: ١٢٠ ألف دينار، تشمل الأجهزة والرواتب.'
    'العمل\محضر اجتماع الإدارة.txt' = 'ناقشنا في الاجتماع مراجعة الميزانية وموعد تسليم التقرير.'
    'العمل\خطة التسويق 2027.docx' = 'خطة التسويق'
    'شخصي\سيرة ذاتية - أحمد علي.pdf' = 'سيرة ذاتية'
    'شخصي\صور عيد ميلاد أحمد.zip' = 'صور'
    'شخصي\عقد إيجار الشقة.pdf' = 'عقد الإيجار'
    'شخصي\تقرير الفحص الطبي.pdf' = 'تقرير طبي'
    'Work\Quarterly report Q3 2026.xlsx' = 'Q3 sales'
    'Work\Annual report 2026 final.pdf' = 'annual report'
    'Work\Weekly status report.docx' = 'status'
    'Work\Invoice 2026-031 Acme.pdf' = 'invoice 031'
    'Work\Invoice 2026-032 Acme.pdf' = 'invoice 032'
    'Work\Project budget.txt' = 'Estimated budget for the new office: 120,000 USD including hardware and salaries.'
    'Work\Meeting notes.txt' = 'We reviewed the budget and agreed on the report deadline.'
    'Personal\Lease agreement.pdf' = 'lease'
    'Personal\Travel plan Istanbul.docx' = 'travel'
}
# Documents get realistic sizes (and pass the duplicate finder's default
# 1 KB minimum); the .txt files keep their readable text.
$seed = 0
foreach ($f in $files.Keys) {
    $p = Join-Path $demo $f
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    $seed++
    if ($f -like '*.txt') { Set-Content -LiteralPath $p -Value $files[$f] -Encoding utf8; continue }
    $bytes = New-Object byte[] (24KB + ($seed * 151KB) % 1900KB)
    (New-Object Random $seed).NextBytes($bytes)
    [IO.File]::WriteAllBytes($p, $bytes)
}
foreach ($pair in @(@('العمل', 'نسخة احتياطية'), @('Work', 'Backup'))) {
    $dst = Join-Path $demo $pair[1]
    New-Item -ItemType Directory -Force $dst | Out-Null
    Get-ChildItem (Join-Path $demo $pair[0]) -File | Where-Object Name -match 'تقرير|report' |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $dst $_.Name) }
}

# ---------------------------------------------------------------- the program
function Check-Crash([string]$When) {
    if (Test-Path $crash) { Get-Content $crash | Write-Host; throw "crash report written ($When)" }
}
function Start-App([string]$Lang) {
    New-Item -Force 'HKCU:\Software\BahthSari' | Out-Null
    Set-ItemProperty 'HKCU:\Software\BahthSari' -Name Language -Value $Lang
    Get-Process bahth -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    $p = Start-Process -FilePath $Exe -PassThru
    $script:appPid = $p.Id
    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 240 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 500; $h = [S]::MainWindow($p.Id) }
    if ($h -eq [IntPtr]::Zero) { throw "no window for $Lang" }
    $script:hwnd = $h
    [void][S]::ShowWindow($h, 3)   # SW_MAXIMIZE
    Focus $h
    return $p
}
function Focus($h) {
    # Windows hands the foreground only to the process that had input last.
    [S]::Chord($VK.ALT)
    [void][S]::SetForegroundWindow($h)
    Start-Sleep -Milliseconds 400
}
# The page's elements, as Windows UI Automation sees them (WebView2 exposes
# the page to it): the first of type $Type whose name matches $Pattern.
function Find-Uia($Type, [string]$Pattern) {
    try {
        $root = [Windows.Automation.AutomationElement]::FromHandle($script:hwnd)
        $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, $Type)
        foreach ($e in $root.FindAll([Windows.Automation.TreeScope]::Descendants, $cond)) { if ($e.Current.Name -match $Pattern) { return $e } }
    } catch { Write-Host "UI Automation: $_" }
    return $null
}
function Uia-Names($Type) {
    try {
        $root = [Windows.Automation.AutomationElement]::FromHandle($script:hwnd)
        $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, $Type)
        return @($root.FindAll([Windows.Automation.TreeScope]::Descendants, $cond) | ForEach-Object { $_.Current.Name } | Select-Object -First 30)
    } catch { return @("(UI Automation failed: $_)") }
}
function Click-Uia($El) {
    $r = $El.Current.BoundingRectangle
    [S]::ClickAt([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2))
}
# The rows of the page's result list ("Results" / «النتائج»).
function Result-Count {
    $list = Find-Uia ([Windows.Automation.ControlType]::List) '^(Results|النتائج)$'
    if (-not $list) { return 0 }
    try {
        $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::ListItem)
        return $list.FindAll([Windows.Automation.TreeScope]::Children, $cond).Count
    } catch { return 0 }
}
# The page has loaded once its search box is there (the window shows
# «جارٍ الفتح…» first).
function Wait-Page {
    for ($i = 0; $i -lt 60; $i++) {
        if (Find-Uia ([Windows.Automation.ControlType]::ComboBox) '.') { Start-Sleep -Seconds 1; return }
        Start-Sleep -Milliseconds 500
    }
    throw 'the page did not load'
}
# Keys go to the page: the window in front, the Alt tap's menu mode
# cancelled, a click in the search box.
function Focus-Page {
    Focus $script:hwnd
    [void][S]::PostMessage($script:hwnd, 0x001F, [IntPtr]::Zero, [IntPtr]::Zero)   # WM_CANCELMODE (the Alt tap)
    Start-Sleep -Milliseconds 300
    $box = Find-Uia ([Windows.Automation.ControlType]::ComboBox) '.'
    if ($box) { Click-Uia $box; Start-Sleep -Milliseconds 300 }
}
# Runs saved search number $Index (0 first) of the page's History menu
# (Ctrl+H, the arrows, Enter), as a person would. The saved searches are
# seeded (Write-History) with the demo folder as their place, so no runner
# folder can show in a shot. Throws unless results come.
function Run-Saved([int]$Index, [string]$What) {
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        Focus-Page
        [S]::Chord($VK.CTRL, $VK.H)
        Start-Sleep -Milliseconds 700
        for ($i = 0; $i -lt $Index; $i++) { [S]::Chord($VK.DOWN); Start-Sleep -Milliseconds 200 }
        [S]::Chord($VK.ENTER)
        for ($i = 0; $i -lt 30 -and (Result-Count) -eq 0; $i++) { Start-Sleep -Milliseconds 500 }
        Start-Sleep -Seconds 2
        $n = Result-Count
        Write-Host "saved search $Index ($What): $n results"
        if ($n -gt 0) {
            $scope = Find-Uia ([Windows.Automation.ControlType]::Button) 'Public.*Demo'
            if (-not $scope) { throw "the search is not limited to $demo; buttons: $((Uia-Names ([Windows.Automation.ControlType]::Button)) -join ' | ')" }
            return
        }
        Write-Host "attempt ${attempt}: no results; menu items: $((Uia-Names ([Windows.Automation.ControlType]::MenuItem)) -join ' | ')"
        [S]::Chord($VK.ESC)
    }
    Warn "no results for $What"
}
# The page's two saved searches for a language: the name search, then the
# one inside documents, both in the demo folder.
function Write-History([string]$Name, [string]$Content) {
    $scope = "$demo\"
    $saved = @(
        [ordered]@{ name = $Name; query = $Name; kind = 0; scope = $scope; content = $false },
        [ordered]@{ name = $Content; query = $Content; kind = 0; scope = $scope; content = $true }
    )
    $json = [ordered]@{ saved = $saved } | ConvertTo-Json -Depth 4 -Compress
    [IO.File]::WriteAllText((Join-Path $data 'history.json'), $json, (New-Object Text.UTF8Encoding $false))
}
function Shot([string]$Name) {
    [S]::Park($W, $H)
    Start-Sleep -Milliseconds 600
    $file = Join-Path $Out $Name
    [S]::Screen($file, $W, $H)
    Write-Host "shot $Name"
}

# The scenes drive the new look's page (UI Automation and the keyboard); the
# one-time questions (the box above the clock, moving the program, rating)
# count as asked, and the move to the new look as done (no "this is the
# new look" line).
New-Item -ItemType Directory -Force $data | Out-Null
[IO.File]::WriteAllText((Join-Path $data 'settings.txt'), "new_ui=1`nnew_ui_default=1`nnew_ui_only=1`nfloating_asked=1`nmove_asked=1`nrating_asked=1`n", (New-Object Text.ASCIIEncoding))

# First start: index the drive (the demo folder with it), as ci/demo.ps1.
Write-Host 'Indexing…'
$p = Start-App 'en'
$started = Get-Date
while (((Get-Date) - $started).TotalMinutes -lt 40 -and -not (Test-Path $index) -and -not $p.HasExited) { Start-Sleep -Seconds 5 }
if (-not (Test-Path $index)) { Check-Crash 'indexing'; throw 'the index was not saved in time' }
Write-Host ("index saved after {0:N0} s" -f ((Get-Date) - $started).TotalSeconds)
Start-Sleep -Seconds 45   # the text of the sample files
Stop-Process -Id $p.Id -Force
Start-Sleep -Seconds 2
# No background indexing of the runner's own documents in the shots'
# status bar: the inside-documents scene reads the demo folder directly.
$settingsFile = Join-Path $data 'settings.txt'
if (Test-Path $settingsFile) {
    $lines = Get-Content $settingsFile -Encoding utf8 | ForEach-Object { if ($_ -match '^content_index=') { 'content_index=0' } else { $_ } }
    Set-Content $settingsFile -Value $lines -Encoding utf8
}

$scenes = @(
    @{ Lang = 'ar'; Name = 'تقرير'; Content = 'الميزانية'; Quick = 'تقرير'; Dupes = @('البحث عن الملفات المكررة'); Options = '(فتح قائمة الخيارات)' },
    @{ Lang = 'en'; Name = 'report'; Content = 'budget'; Quick = 'quarterly report'; Dupes = @('Find duplicate files'); Options = '(Open the Options menu)' }
)
foreach ($s in $scenes) {
    $lang = $s.Lang
    Write-History $s.Name $s.Content
    $p = Start-App $lang
    Wait-Page
    [void][S]::ShowWindow($script:hwnd, 3)

    # 1. Name search (in the demo folder).
    Run-Saved 0 $s.Name
    Shot "1-search-$lang.png"

    # 4. Duplicate finder, on the current results (exact copies in the
    # backup folder), from the Options menu that Settings opens.
    Focus-Page
    [S]::Chord($VK.CTRL, $VK.COMMA)
    Start-Sleep -Seconds 1
    $opt = Find-Uia ([Windows.Automation.ControlType]::Button) $s.Options
    if (-not $opt) { Warn "Options button not found ($lang); buttons: $((Uia-Names ([Windows.Automation.ControlType]::Button)) -join ' | ')" }
    else {
        # A real click: the menu opens at the pointer.
        $menu = $null
        Click-Uia $opt
        for ($i = 0; $i -lt 17 -and -not $menu; $i++) { Start-Sleep -Milliseconds 300; $menu = [S]::TopWindows(0) | Where-Object { [S]::Class($_) -eq '#32768' } | Select-Object -First 1 }
        $item = $null
        if (-not $menu) { Warn "the Options menu did not open ($lang)" }
        else {
            $item = [S]::FindMenuItem($menu, $s.Dupes)
            if (-not $item) { Warn "no duplicate finder item ($lang) among: $([S]::MenuTexts($menu) -join ' / ')" }
        }
        if ($menu) { [S]::Chord($VK.ESC); Start-Sleep -Milliseconds 400 }
        if ($item) {
            # To the new window, which passes Options items to their handlers.
            [void][S]::PostMessage($script:hwnd, 0x0126, [IntPtr]$item[1], [IntPtr]$item[0])   # WM_MENUCOMMAND
            $dupes = $null
            for ($i = 0; $i -lt 20 -and -not $dupes; $i++) {
                Start-Sleep -Milliseconds 500
                $dupes = [S]::TopWindows($script:appPid) | Where-Object { $_ -ne $script:hwnd -and [S]::Class($_) -ne '#32770' -and [S]::Class($_) -notmatch '^InstaFind' } |
                    Where-Object { [S]::Text($_) -match 'المكررة|Duplicate' } | Select-Object -First 1
            }
            if (-not $dupes) { Warn "duplicate finder did not open ($lang)" }
            else {
                [void][S]::ShowWindow($dupes, 3)
                Focus $dupes
                Start-Sleep -Seconds 6   # compares the files' content
                Shot "4-duplicates-$lang.png"
                [void][S]::PostMessage($dupes, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)   # WM_CLOSE
                Start-Sleep -Seconds 1
            }
        }
    }
    # Back from Settings to the results.
    Focus $script:hwnd
    [void][S]::PostMessage($script:hwnd, 0x001F, [IntPtr]::Zero, [IntPtr]::Zero)
    $back = Find-Uia ([Windows.Automation.ControlType]::Button) '^(Back to search|رجوع إلى البحث)'
    if ($back) { Click-Uia $back; Start-Sleep -Milliseconds 500 }
    Check-Crash "duplicates $lang"

    # 2. Inside documents (the second saved search: its words are in the
    # demo folder's .txt files).
    Run-Saved 1 $s.Content
    Shot "2-inside-$lang.png"

    # 3. Quick search over File Explorer showing the demo folder, as a user
    # would call it (and not over the runner's bare desktop).
    Clear-Desktop   # the main window too
    [void][S]::ShowWindow($script:hwnd, 6)   # SW_MINIMIZE
    Start-Process explorer.exe -ArgumentList "`"$(Join-Path $demo $(if ($lang -eq 'ar') { 'العمل' } else { 'Work' }))`""
    $explorer = $null
    for ($i = 0; $i -lt 20 -and -not $explorer; $i++) {
        Start-Sleep -Milliseconds 500
        $explorer = [S]::TopWindows(0) | Where-Object { [S]::Class($_) -eq 'CabinetWClass' } | Select-Object -First 1
    }
    if ($explorer) { [void][S]::ShowWindow($explorer, 3); Focus $explorer; Start-Sleep -Seconds 2 } else { Warn "no File Explorer window ($lang)" }
    [S]::Chord($VK.CTRL, $VK.ALT, $VK.F)
    Start-Sleep -Seconds 2
    [S]::Type($s.Quick)
    Start-Sleep -Seconds 3
    Shot "3-quick-$lang.png"
    [S]::Chord($VK.ESC)
    Start-Sleep -Seconds 1
    if ($explorer) { [void][S]::PostMessage($explorer, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }   # WM_CLOSE

    Check-Crash "scenes $lang"
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
}

$shots = Get-ChildItem $Out -Filter *.png
$shots | ForEach-Object {
    $img = [Drawing.Image]::FromFile($_.FullName)
    Write-Host ("{0}: {1}x{2}, {3:N0} KB" -f $_.Name, $img.Width, $img.Height, ($_.Length / 1KB))
    $img.Dispose()
}
$summary = @("## Store screenshots", "", "Screen: ${W}x${H}; $($shots.Count) shots in the ``store-screenshots`` artifact.")
if ($script:warnings) { $summary += ""; $summary += $script:warnings | ForEach-Object { "- $_" } }
if ($env:GITHUB_STEP_SUMMARY) { $summary | Add-Content $env:GITHUB_STEP_SUMMARY -Encoding utf8 }
if ($shots.Count -lt 8) { throw "only $($shots.Count) of 8 screenshots were taken" }
