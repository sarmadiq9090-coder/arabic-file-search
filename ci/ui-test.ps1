# Automated "real user" test of InstaFind on a real Windows desktop.
#
# Drives the exact published exe the way a person does: real keystrokes and
# mouse clicks (SendInput), the global hotkey, the close button. It covers the
# first-run question, the floating box, Arabic and English search in the main
# window, search inside documents, the right-click and Options menus, the
# quick search popup (apps, answers, files), the Arabic interface and
# close-to-tray. Every step saves a screenshot.
#
# Results are read back with plain Win32 messages (WM_GETTEXT, LVM_GETITEMTEXT,
# SB_GETTEXT, MN_GETHMENU), which work on every Windows without UI Automation.
# The quick popup draws its own rows; with BAHTH_UI_TEST set it writes them to
# quick-rows.txt.
#
# Two kinds of checks:
#   core  - must work; a failure fails the job.
#   probe - a feature we expect to work (typo, wrong keyboard layout, Arabizi,
#           Arabic app names...). A miss is reported as a finding, not a
#           failure, so it can never block a release by itself.
#
# Output: <Out>\report.md (also added to the job summary) and <Out>\*.png.
# Runs under Windows PowerShell 5.1 or PowerShell 7 on 64-bit Windows with the
# x64 exe (reading the list needs the same pointer size).
# This file is UTF-8 with a BOM so Windows PowerShell reads the Arabic in it.
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Out = (Join-Path $env:RUNNER_TEMP 'ui-test')
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
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
    public struct RECT { public int Left, Top, Right, Bottom; }
    delegate bool EnumProc(IntPtr h, IntPtr l);

    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, StringBuilder l);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint item, StringBuilder sb, int n, uint flags);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr VirtualAllocEx(IntPtr p, IntPtr addr, IntPtr size, uint type, uint protect);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr p, IntPtr addr, IntPtr size, uint type);
    [DllImport("kernel32.dll")] static extern bool WriteProcessMemory(IntPtr p, IntPtr addr, byte[] buf, IntPtr n, out IntPtr written);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr p, IntPtr addr, byte[] buf, IntPtr n, out IntPtr read);

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
            System.Threading.Thread.Sleep(30);
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
        System.Threading.Thread.Sleep(80);
        var down = new INPUT { type = 0 }; down.u.mi = new MOUSEINPUT { dwFlags = right ? 8u : 2u };
        var up = new INPUT { type = 0 }; up.u.mi = new MOUSEINPUT { dwFlags = right ? 16u : 4u };
        SendInput(2, new[] { down, up }, Marshal.SizeOf(typeof(INPUT)));
    }

    public static string Text(IntPtr h) {
        var sb = new StringBuilder(4096);
        SendMessage(h, 0x000D /* WM_GETTEXT */, (IntPtr)sb.Capacity, sb);
        return sb.ToString();
    }

    static string Describe(IntPtr h) {
        var c = new StringBuilder(256); GetClassName(h, c, 256);
        RECT r; GetWindowRect(h, out r);
        return h.ToInt64() + "|" + c + "|" + IsWindowVisible(h) + "|" + r.Left + "|" + r.Top + "|" + (r.Right - r.Left) + "|" + (r.Bottom - r.Top) + "|" + Text(h).Replace("|", "/").Replace("\r", " ").Replace("\n", " ");
    }

    // Top-level windows: "handle|class|visible|x|y|w|h|text"; pid 0 = all.
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

    // Items of a popup menu window (#32768).
    public static string[] MenuItems(IntPtr menuWindow) {
        var items = new List<string>();
        IntPtr m = SendMessage(menuWindow, 0x01E1 /* MN_GETHMENU */, IntPtr.Zero, IntPtr.Zero);
        int n = GetMenuItemCount(m);
        for (int i = 0; i < n; i++) {
            var sb = new StringBuilder(512);
            GetMenuString(m, (uint)i, sb, 512, 0x400 /* MF_BYPOSITION */);
            if (sb.Length > 0) items.Add(sb.ToString().Replace("\t", "  "));
        }
        return items.ToArray();
    }

    // Reads text that another process writes into memory we hand it, for list
    // views (LVM_GETITEMTEXTW) and status bars (SB_GETTEXTW).
    static string Remote(IntPtr h, Func<IntPtr, IntPtr, IntPtr, string> ask) {
        uint pid; GetWindowThreadProcessId(h, out pid);
        IntPtr proc = OpenProcess(0x0008 | 0x0010 | 0x0020 | 0x0400, false, pid);
        if (proc == IntPtr.Zero) return null;
        IntPtr mem = VirtualAllocEx(proc, IntPtr.Zero, (IntPtr)8192, 0x3000, 0x04);
        try { return ask(proc, mem, h); }
        finally { VirtualFreeEx(proc, mem, IntPtr.Zero, 0x8000); CloseHandle(proc); }
    }

    static string ReadString(IntPtr proc, IntPtr addr, int bytes) {
        var buf = new byte[bytes]; IntPtr read;
        ReadProcessMemory(proc, addr, buf, (IntPtr)bytes, out read);
        string s = Encoding.Unicode.GetString(buf);
        int z = s.IndexOf('\0');
        return z >= 0 ? s.Substring(0, z) : s;
    }

    public static int ListCount(IntPtr list) {
        return (int)SendMessage(list, 0x1004 /* LVM_GETITEMCOUNT */, IntPtr.Zero, IntPtr.Zero);
    }

    // Row `item`, column `sub` of a list view in another 64-bit process.
    public static string ListText(IntPtr list, int item, int sub) {
        return Remote(list, (proc, mem, h) => {
            var lv = new byte[88];
            BitConverter.GetBytes(1).CopyTo(lv, 0);                         // mask = LVIF_TEXT
            BitConverter.GetBytes(item).CopyTo(lv, 4);                      // iItem
            BitConverter.GetBytes(sub).CopyTo(lv, 8);                       // iSubItem
            BitConverter.GetBytes(mem.ToInt64() + 1024).CopyTo(lv, 24);     // pszText
            BitConverter.GetBytes(2048).CopyTo(lv, 32);                     // cchTextMax
            IntPtr w; WriteProcessMemory(proc, mem, lv, (IntPtr)lv.Length, out w);
            SendMessage(h, 0x1073 /* LVM_GETITEMTEXTW */, (IntPtr)item, mem);
            return ReadString(proc, mem + 1024, 4096);
        });
    }

    public static string StatusText(IntPtr bar, int part) {
        return Remote(bar, (proc, mem, h) => {
            SendMessage(h, 0x040D /* SB_GETTEXTW */, (IntPtr)part, mem);
            return ReadString(proc, mem, 4096);
        });
    }
}
'@

$VK = @{ CTRL = 0x11; ALT = 0x12; SHIFT = 0x10; ENTER = 0x0D; ESC = 0x1B; A = 0x41; F = 0x46; DOWN = 0x28; BACK = 0x08; F4 = 0x73 }
$BM_CLICK = 0x00F5; $BM_GETCHECK = 0x00F0; $WM_CLOSE = 0x0010

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
function Parse-Win([string]$Line) {
    $p = $Line.Split('|', 8)
    [pscustomobject]@{ Handle = [IntPtr][long]$p[0]; Class = $p[1]; Visible = $p[2] -eq 'True'; X = [int]$p[3]; Y = [int]$p[4]; W = [int]$p[5]; H = [int]$p[6]; Text = $p[7] }
}
function Get-Windows([int]$ProcId) { foreach ($l in [U]::Windows($ProcId)) { Parse-Win $l } }
function Get-Children($Hwnd) { foreach ($l in [U]::Children($Hwnd)) { Parse-Win $l } }
function Visible-Window([int]$ProcId, [string]$Class) {
    Get-Windows $ProcId | Where-Object { $_.Visible -and $_.Class -eq $Class } | Select-Object -First 1
}
function Main-Window([int]$ProcId) {
    Get-Windows $ProcId | Where-Object { $_.Visible -and $_.Class -notmatch '^(InstaFind|#32770|tooltips)' -and $_.W -gt 300 } |
        Sort-Object { $_.W * $_.H } -Descending | Select-Object -First 1
}
function Open-Menu {
    Get-Windows 0 | Where-Object { $_.Class -eq '#32768' } | Select-Object -First 1
}
function Focus-Window($Hwnd) {
    # Windows only hands the foreground to the process that last had input;
    # a tap of Alt counts as that.
    [U]::Chord($VK.ALT)
    [void][U]::SetForegroundWindow($Hwnd)
    Start-Sleep -Milliseconds 300
}
function Click-Center($W, [bool]$Right = $false) { [U]::Click([int]($W.X + $W.W / 2), [int]($W.Y + $W.H / 2), $Right) }
function List-Names {
    if (-not $ui.List) { return @() }
    # Enough rows to find the expected file below other good matches.
    $n = [Math]::Min([U]::ListCount($ui.List.Handle), 300)
    $names = @()
    for ($i = 0; $i -lt $n; $i++) { $names += [U]::ListText($ui.List.Handle, $i, 0) }
    return $names
}
function Status-Text {
    if (-not $ui.Status) { return '' }
    return ([U]::StatusText($ui.Status.Handle, 0) + ' ' + [U]::StatusText($ui.Status.Handle, 1)).Trim()
}
# The main window's controls, found by class (and the checkbox by its style).
function Find-Controls($Main) {
    $kids = @(Get-Children $Main.Handle | Where-Object Visible)
    $buttons = @($kids | Where-Object Class -eq 'Button')
    [pscustomobject]@{
        Main    = $Main
        Query   = $kids | Where-Object Class -eq 'Edit' | Sort-Object Y | Select-Object -First 1
        List    = $kids | Where-Object Class -eq 'SysListView32' | Select-Object -First 1
        Status  = $kids | Where-Object Class -eq 'msctls_statusbar32' | Select-Object -First 1
        Content = $buttons | Where-Object { $_.Text -match 'inside documents|داخل المستندات' } | Select-Object -First 1
        Options = $buttons | Where-Object { $_.Text -match '^Options|^خيارات' } | Select-Object -First 1
        All     = $kids
    }
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
Note 'setup' 'screen' ("{0}x{1}, PowerShell {2}" -f [Windows.Forms.SystemInformation]::VirtualScreen.Width, [Windows.Forms.SystemInformation]::VirtualScreen.Height, $PSVersionTable.PSVersion)

# ---------------------------------------------------------------- first start
$t0 = Get-Date
$p = Start-Process -FilePath $Exe -PassThru
$main = Wait-Until { Main-Window $p.Id } 20
$ms = ((Get-Date) - $t0).TotalMilliseconds
Record 'start' 'core' 'main window appears' ($null -ne $main) ("{0:N0} ms" -f $ms)
Record 'start' 'probe' 'window within 3 seconds' ($ms -lt 3000) ("{0:N0} ms" -f $ms)
Shot 'first-start'

# The one-time question about the box above the clock.
$dlg = Wait-Until { Visible-Window $p.Id '#32770' } 40
Record 'first run' 'probe' 'asks about the search box above the clock' ($null -ne $dlg)
if ($dlg) {
    Note 'first run' 'question text' ((Get-Children $dlg.Handle | Where-Object { $_.Class -eq 'Static' -and $_.Text } | ForEach-Object Text) -join ' ')
    Shot 'first-run-question'
    $yes = [U]::GetDlgItem($dlg.Handle, 6)
    if ($yes -ne [IntPtr]::Zero) { [void][U]::PostMessage($yes, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero) } else { Focus-Window $dlg.Handle; [U]::Chord($VK.ENTER) }
    $box = Wait-Until { Visible-Window $p.Id 'InstaFindFloatingBox' } 10
    Record 'floating box' 'core' 'appears after answering Yes' ($null -ne $box)
    Shot 'floating-box'
}
Check-Crash 'first start'

# ---------------------------------------------------------------- index
$t1 = Get-Date
[void](Wait-Until { (Test-Path $index) -or $p.HasExited } 1500)
Record 'index' 'core' 'drives indexed and saved' ((Test-Path $index) -and -not $p.HasExited) ("{0:N0} s" -f ((Get-Date) - $t1).TotalSeconds)
Start-Sleep -Seconds 10
Check-Crash 'indexing'
if ($p.HasExited) { Record 'stability' 'core' 'program still running' $false "exit code $($p.ExitCode)" }

# ---------------------------------------------------------------- main window search
$ui = $null
$main = Main-Window $p.Id
if ($main) {
    $ui = Find-Controls $main
    Note 'main window' 'title' $main.Text
    Note 'main window' 'controls' (($ui.All | ForEach-Object { "$($_.Class):$($_.Text)" } | Select-Object -First 30) -join ' / ')
    Record 'main window' 'core' 'search box, result list and status bar found' ($null -ne $ui.Query -and $null -ne $ui.List -and $null -ne $ui.Status)
}

function Main-Search([string]$Q, [string]$Expect, [string]$Kind, [string]$Check, [int]$Wait = 15) {
    if (-not $ui -or -not $ui.Query) { Record 'search' $Kind $Check $false 'no search box'; return }
    Focus-Window $ui.Main.Handle
    Click-Center $ui.Query
    Start-Sleep -Milliseconds 200
    [U]::Chord($VK.CTRL, $VK.A); [U]::Chord($VK.BACK)
    [U]::Type($Q)
    # A person sees what they type at once; note when the box lags behind.
    $tt = Get-Date
    [void](Wait-Until { [U]::Text($ui.Query.Handle) -eq $Q } 10)
    $lag = ((Get-Date) - $tt).TotalSeconds
    if ($lag -gt 1) { Record 'typing' 'probe' "typed text shows within 1 s ('$Q')" $false ("{0:N1} s; the box shows '{1}'" -f $lag, [U]::Text($ui.Query.Handle)) }
    [U]::Chord($VK.ENTER)
    $t = Get-Date
    $hit = $null
    if ($Expect) { $hit = Wait-Until { List-Names | Where-Object { $_ -like "*$Expect*" } | Select-Object -First 1 } $Wait }
    else { Start-Sleep -Seconds 3 }
    $names = @(List-Names)
    $top = ($names | Select-Object -First 3) -join ' | '
    if ($hit) { $top += " (expected file is row $([array]::IndexOf($names, $hit) + 1))" }
    $count = if ($ui.List) { [U]::ListCount($ui.List.Handle) } else { 0 }
    $detail = "'$Q' -> {0:N1} s, {1} rows; top: {2}; status: {3}" -f ((Get-Date) - $t).TotalSeconds, $count, $top, (Status-Text)
    if ($Expect) { Record 'search' $Kind $Check ($null -ne $hit) $detail } else { Note 'search' $Check $detail }
    Shot ('search-' + ($Check -replace '[^A-Za-z0-9]+', '-').Trim('-'))
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
# 0.9: a search that finds nothing offers what to try next.
$tips = @(Get-Children $ui.Main.Handle | Where-Object { $_.Visible -and $_.Class -eq 'Button' -and $_.Text -match 'Search the web|Search inside file contents|Did you mean' })
Record 'search' 'core' 'no results shows suggestions (search the web / inside contents)' ($tips.Count -gt 0) (($tips | ForEach-Object Text) -join ' | ')

# Inside documents.
if ($ui -and $ui.Content) {
    [void][U]::PostMessage($ui.Content.Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 500
    $on = [U]::SendMessage($ui.Content.Handle, $BM_GETCHECK, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    Record 'search' 'core' 'the "inside documents" box can be ticked' ($on -eq 1)
    # Files changed today: a quick, realistic scope on a fresh install,
    # while the background content index is still being built.
    Main-Search 'dm:today السنوية للشركة' 'تقرير مالي' 'core' 'search inside documents, Arabic text (dm:today)' 90
    Main-Search 'dm:today revenue forecast' 'Quarterly Budget' 'core' 'search inside documents, English text (dm:today)' 90
    Main-Search 'خبرة عشر سنوات' 'سيرة ذاتية' 'probe' 'search inside documents on all drives within 60 s (first day)' 60
    [void][U]::PostMessage($ui.Content.Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
} else { Record 'search' 'core' 'search inside documents' $false 'no "inside documents" checkbox' }

# ---------------------------------------------------------------- menus
if ($ui -and $ui.List) {
    Main-Search 'تقرير مالي' 'تقرير مالي' 'core' 'Arabic file name again before the menus'
    # The first row sits just under the column headers.
    $x = [int]($ui.List.X + $ui.List.W / 2); $y = $ui.List.Y + 36
    [U]::Click($x, $y, $false); Start-Sleep -Milliseconds 300
    [U]::Click($x, $y, $true)
    $menu = Wait-Until { Open-Menu } 5
    Record 'menus' 'core' 'right-click menu on a result' ($null -ne $menu)
    if ($menu) { Note 'menus' 'right-click items' ([U]::MenuItems($menu.Handle) -join ' / ') }
    Shot 'right-click-menu'
    [U]::Chord($VK.ESC); Start-Sleep -Milliseconds 400
}
if ($ui -and $ui.Options) {
    [void][U]::PostMessage($ui.Options.Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
    $menu = Wait-Until { Open-Menu } 5
    Record 'menus' 'core' 'Options menu opens' ($null -ne $menu)
    if ($menu) { Note 'menus' 'Options items' ([U]::MenuItems($menu.Handle) -join ' / ') }
    Shot 'options-menu'
    [U]::Chord($VK.ESC); Start-Sleep -Milliseconds 400
} else { Record 'menus' 'core' 'Options menu opens' $false 'no Options button' }
Check-Crash 'menus'

# ---------------------------------------------------------------- quick search popup
function Quick-Rows {
    if (-not (Test-Path $rowsFile)) { return @() }
    try { return [IO.File]::ReadAllLines($rowsFile, [Text.Encoding]::UTF8) } catch { return @() }
}
# -Best: the expected row must be the top ("Best") result.
function Quick-Search([string]$Q, [string]$Expect, [string]$Kind, [string]$Check, [int]$Wait = 10, [switch]$Best) {
    $like = if ($Best) { "Best`t*$Expect*" } else { "*$Expect*" }
    $pop = Visible-Window $p.Id 'InstaFindQuickSearch'
    if (-not $pop) { [U]::Chord($VK.CTRL, $VK.ALT, $VK.F); $pop = Wait-Until { Visible-Window $p.Id 'InstaFindQuickSearch' } 5 }
    if (-not $pop) { Record 'quick search' $Kind $Check $false 'popup not open'; return }
    [U]::Chord($VK.CTRL, $VK.A); [U]::Chord($VK.BACK)
    [U]::Type($Q)
    $t = Get-Date
    $hit = Wait-Until {
        $rows = @(Quick-Rows)
        if ($rows.Count -gt 0 -and $rows[0] -eq "query`t$Q") { $rows | Select-Object -Skip 1 | Where-Object { $_ -like $like } | Select-Object -First 1 } else { $null }
    } $Wait
    $all = @(Quick-Rows)
    $shown = if ($all.Count -gt 0) { $all[0] -replace "`t", ': ' } else { 'no quick-rows.txt' }
    $rows = $all | Select-Object -Skip 1 | Select-Object -First 4
    Record 'quick search' $Kind $Check ($null -ne $hit) ("'$Q' -> {0:N1} s; {1}; rows: {2}" -f ((Get-Date) - $t).TotalSeconds, $shown, (($rows -replace "`t", ': ') -join ' | '))
    Shot ('quick-' + ($Check -replace '[^A-Za-z0-9]+', '-').Trim('-'))
    Check-Crash "quick search '$Q'"
}

[U]::Chord($VK.CTRL, $VK.ALT, $VK.F)
$pop = Wait-Until { Visible-Window $p.Id 'InstaFindQuickSearch' } 5
Record 'quick search' 'core' 'Ctrl+Alt+F opens the popup' ($null -ne $pop)
Start-Sleep -Seconds 3   # the app catalog loads in the background
Quick-Search 'notepad' 'Notepad' 'core' 'app by English name (notepad)'
Quick-Search 'تقرير مالي' 'تقرير مالي' 'core' 'Arabic file'
Quick-Search '2+2' '4' 'probe' 'calculator answer (2+2)'
Quick-Search 'غوغل كروم' 'Chrome' 'probe' 'app by Arabic name (غوغل كروم) is first' -Best
Quick-Search 'كروم' 'Chrome' 'probe' 'app by short Arabic name (كروم) is first' -Best
Quick-Search 'المفكرة' 'Notepad' 'probe' 'Windows app by Arabic meaning (المفكرة = Notepad) is first' -Best
Quick-Search 'الة حاسبة' 'Calculator' 'probe' 'Windows app by Arabic meaning (الة حاسبة = Calculator) is first' -Best
Quick-Search 'رسام' 'Paint' 'probe' 'Windows app by Arabic meaning (رسام = Paint)'
Quick-Search 'display' 'isplay' 'probe' 'Settings page (display) is first' -Best
Quick-Search 'اعدادات الشاشة' 'isplay' 'probe' 'Settings page in Arabic (اعدادات الشاشة) is first' -Best
Quick-Search 'واي فاي' 'Wi-Fi' 'probe' 'Settings page in Arabic (واي فاي = Wi-Fi)'
Quick-Search 'notepda' 'Notepad' 'core' 'app with a typo (notepda) is first' -Best
Quick-Search '100 usd to iqd' 'IQD' 'core' 'currency answer (100 usd to iqd)' -Best
Quick-Search '100 دولار' 'IQD' 'core' 'currency answer in Arabic (100 دولار)' -Best
Quick-Search '50 يورو بالدينار' 'IQD' 'probe' 'currency answer in Arabic (50 يورو بالدينار)' -Best
Quick-Search 'كوكل كروم' 'Chrome' 'probe' 'Iraqi spelling of an app (كوكل كروم) is first' -Best
Quick-Search 'zzqqxxnotthere' 'Suggest' 'core' 'no results shows a "try" group'
# A sum shows its answer and no files that merely contain the digits.
Quick-Search '2+2' '4' 'core' 'calculator answer (2+2) is first' -Best
$files = @(Quick-Rows | Select-Object -Skip 1 | Where-Object { $_ -like "Files`t*" })
Record 'quick search' 'core' '2+2 shows no unrelated files' ($files.Count -eq 0) (($files -replace "`t", ': ') -join ' | ')
# The empty box lists favourites / recent files (none yet on a fresh runner: it must just not crash).
[U]::Chord($VK.CTRL, $VK.A); [U]::Chord($VK.BACK)
Start-Sleep -Seconds 1
Record 'quick search' 'core' 'empty box after a search does not crash' (-not $p.HasExited)
Shot 'quick-empty'
Quick-Search '5 كم بالميل' '3.1' 'probe' 'unit answer in Arabic (5 كم بالميل)' -Best
[U]::Chord($VK.ESC)
$gone = Wait-Until { -not (Visible-Window $p.Id 'InstaFindQuickSearch') } 3
Record 'quick search' 'core' 'Esc closes the popup' ([bool]$gone)
Check-Crash 'quick search'

# ---------------------------------------------------------------- close to tray
$main = Main-Window $p.Id
if ($main) {
    # What the title bar's X sends.
    [void][U]::PostMessage($main.Handle, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Seconds 3
    $p.Refresh()
    Record 'tray' 'core' 'close button hides to the tray (still running)' (-not $p.HasExited -and -not (Main-Window $p.Id))
    Record 'tray' 'core' 'floating box stays after the close button' ($null -ne (Visible-Window $p.Id 'InstaFindFloatingBox'))
    [U]::Chord($VK.CTRL, $VK.ALT, $VK.F)
    $back = Wait-Until { Visible-Window $p.Id 'InstaFindQuickSearch' } 5
    Record 'tray' 'core' 'Ctrl+Alt+F still works while hidden' ($null -ne $back)
    Shot 'after-close'
    [U]::Chord($VK.ESC)
} else { Record 'tray' 'core' 'close button hides to the tray' $false 'no main window' }
Check-Crash 'close to tray'
if (Test-Path (Join-Path $data 'log.txt')) { Copy-Item (Join-Path $data 'log.txt') (Join-Path $Out 'log-en.txt') }
Get-Process bahth* -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

# ---------------------------------------------------------------- Arabic interface
New-Item 'HKCU:\Software\BahthSari' -Force | Out-Null
Set-ItemProperty 'HKCU:\Software\BahthSari' -Name Language -Value 'ar'
$p = Start-Process -FilePath $Exe -PassThru
$main = Wait-Until { Main-Window $p.Id } 20
Record 'arabic UI' 'core' 'starts in Arabic' ($null -ne $main -and $main.Text -match '[؀-ۿ]') $(if ($main) { $main.Text } else { '' })
Start-Sleep -Seconds 4
Shot 'arabic-main'
if ($main) {
    $ui = Find-Controls $main
    Main-Search 'تقرير' 'تقرير مالي' 'core' 'Arabic UI search'
    if ($ui.Options) {
        [void][U]::PostMessage($ui.Options.Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
        $menu = Wait-Until { Open-Menu } 5
        if ($menu) { Note 'arabic UI' 'Options items' ([U]::MenuItems($menu.Handle) -join ' / ') }
        Shot 'arabic-options-menu'
        [U]::Chord($VK.ESC); Start-Sleep -Milliseconds 400
    }
}
Quick-Search 'تقرير' 'تقرير مالي' 'core' 'Arabic UI quick search'
Quick-Search 'notepad' 'Notepad' 'probe' 'Arabic UI app search'
[U]::Chord($VK.ESC)
Start-Sleep -Seconds 1

# Alt+F4 on the main window: same as the close button.
$main = Main-Window $p.Id
if ($main) {
    Focus-Window $main.Handle
    [U]::Chord($VK.ALT, $VK.F4)
    Start-Sleep -Seconds 3
    $p.Refresh()
    Record 'tray' 'core' 'Alt+F4 hides to the tray (still running)' (-not $p.HasExited -and -not (Main-Window $p.Id))
    Record 'tray' 'core' 'floating box stays after Alt+F4' ($null -ne (Visible-Window $p.Id 'InstaFindFloatingBox'))
    Shot 'after-alt-f4'
    Check-Crash 'Alt+F4'
}

# Clicking the floating box over and over, as an impatient user does.
$box = Visible-Window $p.Id 'InstaFindFloatingBox'
if ($box) {
    for ($i = 0; $i -lt 12; $i++) {
        $b = Visible-Window $p.Id 'InstaFindFloatingBox'
        if ($b) { Click-Center $b }
        Start-Sleep -Milliseconds 250
    }
    Start-Sleep -Seconds 2
    $p.Refresh()
    Record 'floating box' 'core' 'still running after 12 quick clicks on the box' (-not $p.HasExited) $(if ($p.HasExited) { "exit code $($p.ExitCode)" } else { '' })
    Record 'floating box' 'core' 'box still shown after 12 quick clicks' ($null -ne (Visible-Window $p.Id 'InstaFindFloatingBox'))
    Shot 'after-box-clicks'
    [U]::Chord($VK.ESC)
    Check-Crash 'clicking the floating box'
} else { Record 'floating box' 'core' 'box shown in the Arabic run' $false 'not visible' }
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
    if ($d.Length -gt 500) { $d = $d.Substring(0, 500) + '…' }
    [void]$md.AppendLine("| $($r.Result) | $($r.Area) | $($r.Check) | $d |")
}
[void]$md.AppendLine('')
[void]$md.AppendLine('MISS = a feature we expect that did not work; it does not fail the job. Screenshots are in the ui-test artifact.')
[IO.File]::WriteAllText((Join-Path $Out 'report.md'), $md.ToString(), (New-Object Text.UTF8Encoding $false))
if ($env:GITHUB_STEP_SUMMARY) { [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $md.ToString(), (New-Object Text.UTF8Encoding $false)) }
Write-Host $md.ToString()
if ($fail -gt 0) { exit 1 }
