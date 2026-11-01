# TEMPORARY: screenshots the Arabic match underline on real Windows under each
# BAHTH_DRAW_MODE, with the measured values logged, to pick the right mapping.
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$index = Join-Path $data 'index.bin'

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit, Size = 40)] struct INPUT {
        [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; [FieldOffset(8)] public MOUSEINPUT mi;
    }
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int hh, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, string l);
    [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    static INPUT Key(ushort vk, ushort scan, uint flags) {
        var i = new INPUT(); i.type = 1; i.ki.vk = vk; i.ki.scan = scan; i.ki.flags = flags; return i;
    }
    static INPUT Mouse(uint flags) { var i = new INPUT(); i.type = 0; i.mi.flags = flags; return i; }
    public static string Class(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
    public static string Text(IntPtr h) { var s = new StringBuilder(512); GetWindowText(h, s, 512); return s.ToString(); }
    // The process's largest visible top-level window.
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
    public static List<IntPtr> Children(IntPtr parent) {
        var list = new List<IntPtr>();
        EnumChildWindows(parent, (h, l) => { list.Add(h); return true; }, IntPtr.Zero);
        return list;
    }
    public static void Click(IntPtr h) {
        RECT r; GetWindowRect(h, out r);
        SetCursorPos((r.L + r.R) / 2, (r.T + r.B) / 2);
        SendInput(2, new[] { Mouse(2), Mouse(4) }, Marshal.SizeOf(typeof(INPUT)));
        SetCursorPos(2000, 2000);
    }
    public static void SetText(IntPtr h, string t) { SendMessage(h, 0x000C, IntPtr.Zero, t); }
    public static bool Checked(IntPtr h) { return SendMessage(h, 0x00F0, IntPtr.Zero, IntPtr.Zero) == (IntPtr)1; }
    // Types one character as Unicode input, so Arabic needs no keyboard layout.
    public static void Char(char c) {
        SendInput(2, new[] { Key(0, c, 4), Key(0, c, 4 | 2) }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void VKey(ushort vk) {
        SendInput(2, new[] { Key(vk, 0, 0), Key(vk, 0, 2) }, Marshal.SizeOf(typeof(INPUT)));
    }
    // The visible frame, without the invisible resize borders.
    public static void Snap(IntPtr h, string path) {
        RECT r;
        if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) != 0) GetWindowRect(h, out r);
        using (var bmp = new Bitmap(r.R - r.L, r.B - r.T))
        using (var g = Graphics.FromImage(bmp)) {
            g.CopyFromScreen(r.L, r.T, 0, 0, bmp.Size);
            bmp.Save(path, ImageFormat.Png);
        }
    }
}
'@

# Sample files. Empty documents are enough for name search; the two text files
# carry the words the "inside documents" scene looks for.
$docs = [Environment]::GetFolderPath('MyDocuments')
$files = @{
    'العمل\تقرير المبيعات 2026.xlsx' = ''

$docs = [Environment]::GetFolderPath('MyDocuments')
foreach ($f in @('العمل\تقرير المبيعات 2026.xlsx', 'العمل\التقرير السنوي النهائي.pdf', 'العمل\تقرير الاجتماع الأسبوعي.docx', 'Work\Quarterly report Q3 2026.xlsx')) {
    $p = Join-Path $docs $f
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    Set-Content -LiteralPath $p -Value '' -Encoding utf8
}

function Start-App([string]$lang) {
    New-Item -Force 'HKCU:\Software\BahthSari' | Out-Null
    Set-ItemProperty 'HKCU:\Software\BahthSari' -Name Language -Value $lang
    $p = Start-Process -FilePath $Exe -PassThru
    $deadline = (Get-Date).AddMinutes(2)
    $h = [IntPtr]::Zero
    while ((Get-Date) -lt $deadline -and $h -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 500; $h = [Win]::MainWindow($p.Id) }
    if ($h -eq [IntPtr]::Zero) { throw "no window" }
    [Win]::ShowWindow($h, 9) | Out-Null
    [Win]::SetWindowPos($h, [IntPtr]::Zero, 8, 8, 960, 600, 0x0040) | Out-Null
    [Win]::SetForegroundWindow($h) | Out-Null
    return @($p, $h)
}

Write-Host 'Indexing…'
$p, $h = Start-App 'en'
$deadline = (Get-Date).AddMinutes(30)
while ((Get-Date) -lt $deadline -and -not (Test-Path $index)) { Start-Sleep -Seconds 5 }
Stop-Process -Id $p.Id -Force
Start-Sleep -Seconds 2

$env:BAHTH_DRAW_LOG = Join-Path $Out 'draw.log'
foreach ($run in @(@('ar', 0), @('ar', 1), @('ar', 2), @('ar', 3), @('en', 0))) {
    $lang, $mode = $run
    $env:BAHTH_DRAW_MODE = "$mode"
    Add-Content -LiteralPath $env:BAHTH_DRAW_LOG -Value "=== $lang mode $mode"
    $p, $h = Start-App $lang
    Start-Sleep -Seconds 4
    $box = [Win]::Children($h) | Where-Object { [Win]::Class($_) -eq 'Edit' } | Select-Object -First 1
    [Win]::SetText($box, $(if ($lang -eq 'ar') { 'التقرير' } else { 'report 2026' }))
    Start-Sleep -Seconds 3
    [Win]::Snap($h, (Join-Path $Out "$lang-mode$mode.png"))
    Stop-Process -Id $p.Id -Force
    Start-Sleep -Seconds 2
}
Get-ChildItem $Out | Format-Table Name, Length
