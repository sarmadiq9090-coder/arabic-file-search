# Records the README demos on a real Windows machine: creates a few sample
# files, starts the program in Arabic and then in English, types searches into
# it and saves a frame of its window every 125 ms (8 frames per second), plus
# a still screenshot of each scene. ci/make-media.sh turns the frames into GIFs.
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

Add-Type -AssemblyName System.Drawing, System.Windows.Forms, UIAutomationClient, UIAutomationTypes
Add-Type -ReferencedAssemblies System.Drawing @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit, Size = 40)] struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int hh, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    static INPUT Key(ushort vk, ushort scan, uint flags) {
        var i = new INPUT(); i.type = 1; i.ki.vk = vk; i.ki.scan = scan; i.ki.flags = flags; return i;
    }
    // Types one character as Unicode input, so Arabic needs no keyboard layout.
    public static void Char(char c) {
        SendInput(2, new[] { Key(0, c, 4), Key(0, c, 4 | 2) }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void VKey(ushort vk) {
        SendInput(2, new[] { Key(vk, 0, 0), Key(vk, 0, 2) }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Snap(IntPtr h, string path) {
        RECT r; GetWindowRect(h, out r);
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
    'العمل\التقرير السنوي النهائي.pdf' = ''
    'العمل\تقرير الاجتماع الأسبوعي.docx' = ''
    'العمل\ميزانية المشروع.txt' = 'الميزانية التقديرية للمشروع الجديد: ١٢٠ ألف دينار، تشمل الأجهزة والرواتب.'
    'العمل\محضر اجتماع الإدارة.txt' = 'ناقشنا في الاجتماع مراجعة الميزانية وموعد تسليم التقرير.'
    'شخصي\سيرة ذاتية - أحمد علي.pdf' = ''
    'شخصي\صور عيد ميلاد أحمد.zip' = ''
    'شخصي\عقد إيجار الشقة.pdf' = ''
    'Work\Quarterly report Q3 2026.xlsx' = ''
    'Work\Annual report 2026 final.pdf' = ''
    'Work\Invoice 2026-031 Acme.pdf' = ''
    'Work\Invoice 2026-032 Acme.pdf' = ''
    'Work\Project budget.txt' = 'Estimated budget for the new office: 120,000 USD including hardware and salaries.'
    'Work\Meeting notes.txt' = 'We reviewed the budget and agreed on the report deadline.'
    'Personal\Lease agreement.pdf' = ''
}
foreach ($f in $files.Keys) {
    $p = Join-Path $docs $f
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    Set-Content -LiteralPath $p -Value $files[$f] -Encoding utf8
}

$script:frame = 0
$script:hwnd = [IntPtr]::Zero
$script:dir = $null

function Start-App([string]$lang) {
    New-Item -Force 'HKCU:\Software\BahthSari' | Out-Null
    Set-ItemProperty 'HKCU:\Software\BahthSari' -Name Language -Value $lang
    $p = Start-Process -FilePath $Exe -PassThru
    $deadline = (Get-Date).AddMinutes(2)
    while ((Get-Date) -lt $deadline) {
        $p.Refresh()
        if ($p.MainWindowHandle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 500
    }
    if ($p.MainWindowHandle -eq [IntPtr]::Zero) { throw "no window for $lang" }
    $script:hwnd = $p.MainWindowHandle
    [Win]::ShowWindow($script:hwnd, 9) | Out-Null   # SW_RESTORE
    [Win]::SetWindowPos($script:hwnd, [IntPtr]::Zero, 8, 8, 960, 600, 0x0040) | Out-Null
    [Win]::SetForegroundWindow($script:hwnd) | Out-Null
    return $p
}

function Element([string]$type) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:hwnd)
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::$type)
    return $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
}

function Snap { [Win]::Snap($script:hwnd, (Join-Path $script:dir ('{0:D4}.png' -f $script:frame))); $script:frame++ }
function Hold([double]$seconds) {
    for ($i = 0; $i -lt [int]($seconds * 8); $i++) { Start-Sleep -Milliseconds 110; Snap }
}
function Still([string]$name) { [Win]::Snap($script:hwnd, (Join-Path $Out $name)) }

function Query-Box { (Element 'Edit')[0] }
function Clear-Query {
    $box = Query-Box
    $box.SetFocus()
    $box.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('')
    Hold 0.5
}
function Type-Text([string]$text) {
    (Query-Box).SetFocus()
    foreach ($c in $text.ToCharArray()) { [Win]::Char($c); Start-Sleep -Milliseconds 90; Snap }
}
function Set-Inside([bool]$on, [string[]]$names) {
    foreach ($cb in (Element 'CheckBox')) {
        if ($names -contains $cb.Current.Name) {
            $t = $cb.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
            $want = if ($on) { 'On' } else { 'Off' }
            if ($t.Current.ToggleState.ToString() -ne $want) { $t.Toggle() }
            return
        }
    }
    Write-Host "::warning::checkbox not found: $($names -join ' / ')"
}

function Record([string]$lang, [scriptblock]$scenes) {
    $script:frame = 0
    $script:dir = Join-Path $Out "frames-$lang"
    New-Item -ItemType Directory -Force $script:dir | Out-Null
    $p = Start-App $lang
    Start-Sleep -Seconds 4
    & $scenes
    Stop-Process -Id $p.Id -Force
    Start-Sleep -Seconds 2
}

# First start: let the program index the drive (several minutes on a runner).
Write-Host 'Indexing…'
$p = Start-App 'en'
$deadline = (Get-Date).AddMinutes(30)
while ((Get-Date) -lt $deadline -and -not (Test-Path $index)) { Start-Sleep -Seconds 5 }
if (-not (Test-Path $index)) { throw 'the index was not saved in time' }
Start-Sleep -Seconds 30   # content indexing of the sample files
Stop-Process -Id $p.Id -Force
Start-Sleep -Seconds 2

$insideAr = @('البحث داخل المستندات أيضاً')
$insideEn = @('Also search inside documents')

Record 'ar' {
    Set-Inside $false $insideAr
    Hold 1
    Type-Text 'احمد';      Hold 2.5
    Clear-Query
    Type-Text 'التقرير';   Hold 2.5; Still 'search-ar.png'
    Clear-Query
    Type-Text 'hgjrvdv';   Hold 3
    Clear-Query
    Set-Inside $true $insideAr; Hold 0.8
    Type-Text 'الميزانية'; [Win]::VKey(0x0D); Hold 4; Still 'inside-ar.png'
    Set-Inside $false $insideAr
}

Record 'en' {
    Set-Inside $false $insideEn
    Hold 1
    Type-Text 'invoice';     Hold 2.5
    Clear-Query
    Type-Text 'report 2026'; Hold 2.5; Still 'search-en.png'
    Clear-Query
    Set-Inside $true $insideEn; Hold 0.8
    Type-Text 'budget'; [Win]::VKey(0x0D); Hold 4; Still 'inside-en.png'
    Set-Inside $false $insideEn
}

Get-ChildItem $Out | Format-Table Name, Length
$crash = Join-Path $data 'crash.txt'
if (Test-Path $crash) { Get-Content $crash | Write-Host; throw 'crash report written' }
