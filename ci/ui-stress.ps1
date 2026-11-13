# UI stress test of InstaFind on a real Windows desktop: tries to break the
# program the way an impatient person would, in one of its two looks.
#
#   -Look classic   the Win32 window (BAHTH_CLASSIC=1: since 1.0 users only
#                   get the new look and new_ui=0 is ignored; the variable is
#                   the program's own switch for the tests, and the copies
#                   the program starts itself inherit it)
#   -Look new       the WebView2 "new look" (new_ui=1), plus: no way back to
#                   the classic look, WebView2 killed (the classic window for
#                   one run, then "Try the new look"), a read-only profile
#
# What it does, in this order (ci/ui-stress.ps1 is not a release gate; the
# gates are ci/ui-test.ps1 and ci/newlook-test.ps1, which it copies helpers
# from and never touches):
#   A  typing storms while the first index is still being built
#   B  window abuse: tray, minimise/maximise/resize/move, Alt+F4 and X, the
#      Ctrl+Alt+F popup, the floating box, the Options menu, its toggles and
#      its dialogs
#   C  waits for the first index (bounded)
#   D  right results for files that exist; filters, sorting, search inside
#      documents, stop
#   F  (new look) no way back to the classic look in the menus; WebView2
#      killed once and twice; the one-run fallback and back several times;
#      read-only WebView2 folder
#   E  language switched Arabic <-> English
#   D2 typing while files are created, renamed and deleted in the background
#   D3 actions on results that were deleted or renamed meanwhile, 30 files
#      opened quickly, the right-click menu
#   G  five idle minutes, memory, one process, a last search, exit from the tray
#
# After every step: crash.txt, the process, and whether the window still
# answers (SendMessageTimeout, IsHungAppWindow). Memory of every bahth
# process and of its WebView2 children is sampled every 5 s to memory.csv.
#
# Two kinds of checks:
#   core  - a crash report, the program exiting, a hang over 10 s, or a wrong
#           result for a file that exists. A failure fails the job.
#   probe - a finding (MISS): reported, never fails the job. Problems of the
#           test script itself are reported as probes too, marked SCRIPT.
#
# Limits it keeps (GitHub's rules first): a time budget (-MaxMinutes) with a
# watchdog that ends the run before the job's timeout; at least 10 GB of free
# disk; no network (the program's update check and exchange-rate download are
# pre-seeded off, WebView2's background requests are switched off); it only
# ends processes it started (bahth, its WebView2 children, the editor windows
# it opened) and deletes what it created.
#
# Output in <Out>: report.md (also the job summary), run.log (everything),
# progress.tsv, memory.csv, crash.txt (if any), log.txt (the program's log),
# settings.txt, NN-name.png screenshots.
# Windows PowerShell 5.1, 64-bit Windows, the x64 exe (reading the result list
# needs the same pointer size). UTF-8 with a BOM: it holds Arabic text.
param(
    [Parameter(Mandatory)][string]$Exe,
    [ValidateSet('classic', 'new')][string]$Look = 'classic',
    [string]$Out = (Join-Path $env:RUNNER_TEMP 'ui-stress'),
    # Everything, the idle minutes included, must fit in this; scenarios that
    # no longer fit are skipped and reported. The job's timeout is 45.
    [int]$MaxMinutes = 37,
    # How long to wait for the first index, counted from the program's start.
    [int]$IndexWaitMinutes = 12,
    [int]$IdleMinutes = 5,
    [int]$Seed = 20261111,
    # Leave WebView2's own background requests on (off by default here).
    [switch]$AllowWebViewNetwork
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

// Input, windows, menus, lists (copied from ci/ui-test.ps1 and
// ci/newlook-test.ps1, with time limits on every message), the memory
// sampler, the file churn and the watchdog. C# 5 only.
public static class S {
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }
    public struct RECT { public int Left, Top, Right, Bottom; }
    public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct NOTIFYICONIDENTIFIER { public uint cbSize; public IntPtr hWnd; public uint uID; public Guid guidItem; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct PROCESSENTRY32 {
        public uint dwSize; public uint cntUsage; public uint th32ProcessID; public IntPtr th32DefaultHeapID; public uint th32ModuleID;
        public uint cntThreads; public uint th32ParentProcessID; public int pcPriClassBase; public uint dwFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string szExeFile;
    }
    delegate bool EnumProc(IntPtr h, IntPtr l);

    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsHungAppWindow(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageTimeoutW")] static extern IntPtr SendMessageTimeoutText(IntPtr h, uint msg, IntPtr w, StringBuilder l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW")] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "PostMessageW")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint item, StringBuilder sb, int n, uint flags);
    [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr m, uint item, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetSubMenu(IntPtr m, int pos);
    [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr hwnd, IntPtr m, uint item, out RECT r);
    [DllImport("user32.dll")] static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("shell32.dll")] static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT r);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr VirtualAllocEx(IntPtr p, IntPtr addr, IntPtr size, uint type, uint protect);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr p, IntPtr addr, IntPtr size, uint type);
    [DllImport("kernel32.dll")] static extern bool WriteProcessMemory(IntPtr p, IntPtr addr, byte[] buf, IntPtr n, out IntPtr written);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr p, IntPtr addr, byte[] buf, IntPtr n, out IntPtr read);
    [DllImport("kernel32.dll")] static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint pid);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, EntryPoint = "Process32FirstW")] static extern bool Process32First(IntPtr snap, ref PROCESSENTRY32 e);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, EntryPoint = "Process32NextW")] static extern bool Process32Next(IntPtr snap, ref PROCESSENTRY32 e);

    const uint KEYUP = 2, UNICODE = 4, EXTENDED = 1;

    // ------------------------------------------------------------ input
    static INPUT Key(ushort vk, ushort scan, uint flags) {
        // Arrows, Insert, Delete, Home, End, Page Up/Down are extended keys.
        if (scan == 0 && ((vk >= 0x21 && vk <= 0x28) || vk == 0x2D || vk == 0x2E)) flags |= EXTENDED;
        var i = new INPUT { type = 1 };
        i.u.ki = new KEYBDINPUT { wVk = vk, wScan = scan, dwFlags = flags };
        return i;
    }

    // Types text with real key events, `chunk` characters at a time, pausing
    // `pauseMs` between chunks (0: as fast as Windows takes them).
    public static void Type(string text, int chunk, int pauseMs) {
        if (string.IsNullOrEmpty(text)) return;
        int size = Marshal.SizeOf(typeof(INPUT));
        if (chunk < 1) chunk = 1;
        var batch = new List<INPUT>();
        for (int i = 0; i < text.Length; i++) {
            char c = text[i];
            batch.Add(Key(0, c, UNICODE));
            batch.Add(Key(0, c, UNICODE | KEYUP));
            if (batch.Count >= chunk * 2 || i == text.Length - 1) {
                SendInput((uint)batch.Count, batch.ToArray(), size);
                batch.Clear();
                if (pauseMs > 0) Thread.Sleep(pauseMs);
            }
        }
    }

    // Presses a chord such as Ctrl+Alt+F: modifiers down, key, all up.
    public static void Chord(params ushort[] vks) {
        var list = new List<INPUT>();
        foreach (var v in vks) list.Add(Key(v, 0, 0));
        for (int i = vks.Length - 1; i >= 0; i--) list.Add(Key(vks[i], 0, KEYUP));
        SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
    }

    // The same key `times` times, `pauseMs` apart.
    public static void Tap(ushort vk, int times, int pauseMs) {
        int size = Marshal.SizeOf(typeof(INPUT));
        for (int i = 0; i < times; i++) {
            SendInput(2, new[] { Key(vk, 0, 0), Key(vk, 0, KEYUP) }, size);
            if (pauseMs > 0) Thread.Sleep(pauseMs);
        }
    }

    // A key held down: repeated key-down events, one key-up at the end.
    public static void Hold(ushort vk, int ms, int perSecond) {
        int size = Marshal.SizeOf(typeof(INPUT));
        int gap = Math.Max(5, 1000 / Math.Max(1, perSecond));
        var sw = Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < ms) {
            SendInput(1, new[] { Key(vk, 0, 0) }, size);
            Thread.Sleep(gap);
        }
        SendInput(1, new[] { Key(vk, 0, KEYUP) }, size);
    }

    public static void MoveCursor(int x, int y) { SetCursorPos(x, y); }

    public static void Click(int x, int y, bool right) {
        SetCursorPos(x, y);
        Thread.Sleep(60);
        var down = new INPUT { type = 0 }; down.u.mi = new MOUSEINPUT { dwFlags = right ? 8u : 2u };
        var up = new INPUT { type = 0 }; up.u.mi = new MOUSEINPUT { dwFlags = right ? 16u : 4u };
        SendInput(2, new[] { down, up }, Marshal.SizeOf(typeof(INPUT)));
    }

    // Presses the left button at (x1, y1), moves to (x2, y2) in steps, releases.
    public static void Drag(int x1, int y1, int x2, int y2) {
        int size = Marshal.SizeOf(typeof(INPUT));
        SetCursorPos(x1, y1);
        Thread.Sleep(80);
        var down = new INPUT { type = 0 }; down.u.mi = new MOUSEINPUT { dwFlags = 2u };
        SendInput(1, new[] { down }, size);
        for (int i = 1; i <= 12; i++) {
            Thread.Sleep(25);
            SetCursorPos(x1 + (x2 - x1) * i / 12, y1 + (y2 - y1) * i / 12);
            // A real (relative) move, so the window's move loop sees it.
            var mv = new INPUT { type = 0 }; mv.u.mi = new MOUSEINPUT { dx = (i % 2 == 0) ? 1 : -1, dy = 0, dwFlags = 1u };
            SendInput(1, new[] { mv }, size);
        }
        Thread.Sleep(60);
        var up = new INPUT { type = 0 }; up.u.mi = new MOUSEINPUT { dwFlags = 4u };
        SendInput(1, new[] { up }, size);
    }

    // ------------------------------------------------------------ messages
    // SendMessage with a 5 s limit; zero when the window does not answer.
    static IntPtr Send(IntPtr h, uint msg, IntPtr w, IntPtr l) {
        IntPtr r;
        if (SendMessageTimeout(h, msg, w, l, 2 /* SMTO_ABORTIFHUNG */, 5000, out r) == IntPtr.Zero) return IntPtr.Zero;
        return r;
    }
    public static long Msg(IntPtr h, int msg, long w, long l) { return Send(h, (uint)msg, (IntPtr)w, (IntPtr)l).ToInt64(); }
    public static bool Post(IntPtr h, int msg, long w, long l) { return PostMessage(h, (uint)msg, (IntPtr)w, (IntPtr)l); }

    // How long the window's thread takes to answer WM_NULL, in ms.
    // -1: no answer within `ms` (hung); -2: no such window; -3: refused.
    public static int Ping(IntPtr h, int ms) {
        if (!IsWindow(h)) return -2;
        var sw = Stopwatch.StartNew();
        IntPtr r;
        IntPtr ok = SendMessageTimeout(h, 0 /* WM_NULL */, IntPtr.Zero, IntPtr.Zero, 0 /* SMTO_NORMAL */, (uint)ms, out r);
        int took = (int)sw.ElapsedMilliseconds;
        if (ok != IntPtr.Zero) return took;
        if (!IsWindow(h)) return -2;
        return took >= ms - 250 ? -1 : -3;
    }

    public static string Text(IntPtr h) {
        var sb = new StringBuilder(4096);
        IntPtr r;
        SendMessageTimeoutText(h, 0x000D /* WM_GETTEXT */, (IntPtr)sb.Capacity, sb, 2, 1000, out r);
        return sb.ToString();
    }
    public static int TextLength(IntPtr h) { return (int)Msg(h, 0x000E /* WM_GETTEXTLENGTH */, 0, 0); }

    public static int Pid(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return (int)p; }

    // ------------------------------------------------------------ windows
    // "handle|class|visible|x|y|w|h|text"
    public static string Describe(IntPtr h) {
        var c = new StringBuilder(256); GetClassName(h, c, 256);
        RECT r; GetWindowRect(h, out r);
        return h.ToInt64() + "|" + c + "|" + IsWindowVisible(h) + "|" + r.Left + "|" + r.Top + "|" + (r.Right - r.Left) + "|" + (r.Bottom - r.Top) + "|" + Text(h).Replace("|", "/").Replace("\r", " ").Replace("\n", " ");
    }

    // Top-level windows of process `pid` (0: every visible one).
    public static string[] Windows(int pid) {
        var found = new List<string>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (pid == 0 ? IsWindowVisible(h) : p == pid) found.Add(Describe(h));
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    // Handles of the visible top-level windows (to tell later which are new).
    public static long[] VisibleHandles() {
        var found = new List<long>();
        EnumWindows((h, l) => { if (IsWindowVisible(h)) found.Add(h.ToInt64()); return true; }, IntPtr.Zero);
        return found.ToArray();
    }

    public static string[] Children(IntPtr parent) {
        var found = new List<string>();
        EnumChildWindows(parent, (h, l) => { found.Add(Describe(h)); return true; }, IntPtr.Zero);
        return found.ToArray();
    }

    public static long DlgItem(IntPtr dialog, int id) { return GetDlgItem(dialog, id).ToInt64(); }

    // The client area in screen pixels: x, y, width, height. Both corners: in
    // a right-to-left (mirrored) window the client origin is the top RIGHT.
    public static int[] ClientOnScreen(IntPtr h) {
        RECT r; GetClientRect(h, out r);
        var a = new POINT(); a.X = r.Left; a.Y = r.Top;
        var b = new POINT(); b.X = r.Right; b.Y = r.Bottom;
        ClientToScreen(h, ref a);
        ClientToScreen(h, ref b);
        return new[] { Math.Min(a.X, b.X), Math.Min(a.Y, b.Y), Math.Abs(b.X - a.X), Math.Abs(b.Y - a.Y) };
    }

    // (Asynchronous: a window that hangs must not hang the test with it.)
    public static void Place(IntPtr h, int x, int y, int w, int ht) { SetWindowPos(h, IntPtr.Zero, x, y, w, ht, 0x0004 | 0x0010 | 0x4000 /* NOZORDER | NOACTIVATE | ASYNCWINDOWPOS */); }

    // The tray icon's rectangle on screen (icon 0 of `hwnd`), or null.
    public static int[] TrayIconRect(IntPtr hwnd) {
        var id = new NOTIFYICONIDENTIFIER();
        id.cbSize = (uint)Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER));
        id.hWnd = hwnd; id.uID = 0;
        RECT r;
        try { if (Shell_NotifyIconGetRect(ref id, out r) != 0) return null; } catch (Exception) { return null; }
        return new[] { r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top };
    }

    // ------------------------------------------------------------ menus
    static IntPtr MenuOf(IntPtr menuWindow) { return Send(menuWindow, 0x01E1 /* MN_GETHMENU */, IntPtr.Zero, IntPtr.Zero); }

    // Items of a popup menu window (#32768): "pos|state|submenu|menu|text".
    public static string[] MenuDump(IntPtr menuWindow) {
        var items = new List<string>();
        IntPtr m = MenuOf(menuWindow);
        if (m == IntPtr.Zero) return items.ToArray();
        int n = GetMenuItemCount(m);
        for (int i = 0; i < n; i++) {
            var sb = new StringBuilder(512);
            GetMenuString(m, (uint)i, sb, 512, 0x400 /* MF_BYPOSITION */);
            uint state = GetMenuState(m, (uint)i, 0x400);
            IntPtr sub = GetSubMenu(m, i);
            if (state == 0xFFFFFFFF) state = 0;
            // (For a submenu the high byte counts its items.)
            if (sub != IntPtr.Zero) state &= 0xFF;
            items.Add(i + "|" + state + "|" + sub.ToInt64() + "|" + m.ToInt64() + "|" + sb.ToString().Replace("\t", "  ").Replace("|", "/"));
        }
        return items.ToArray();
    }

    // Where item `pos` of the open popup menu is on screen: x, y, w, h.
    public static int[] MenuItemRect(IntPtr menuWindow, int pos) {
        IntPtr m = MenuOf(menuWindow);
        if (m == IntPtr.Zero) return null;
        RECT r;
        if (!GetMenuItemRect(IntPtr.Zero, m, (uint)pos, out r) && !GetMenuItemRect(menuWindow, m, (uint)pos, out r)) return null;
        RECT w; GetWindowRect(menuWindow, out w);
        int cx = (r.Left + r.Right) / 2, cy = (r.Top + r.Bottom) / 2;
        if (cx < w.Left || cx > w.Right || cy < w.Top || cy > w.Bottom) return null;
        return new[] { r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top };
    }

    // ------------------------------------------------------------ lists
    // Reads what another process writes into memory we hand it, for list
    // views (LVM_GETITEMTEXTW), headers and status bars (SB_GETTEXTW).
    static string Remote(IntPtr h, Func<IntPtr, IntPtr, IntPtr, string> ask) {
        uint pid; GetWindowThreadProcessId(h, out pid);
        IntPtr proc = OpenProcess(0x0008 | 0x0010 | 0x0020 | 0x0400, false, pid);
        if (proc == IntPtr.Zero) return null;
        IntPtr mem = VirtualAllocEx(proc, IntPtr.Zero, (IntPtr)8192, 0x3000, 0x04);
        if (mem == IntPtr.Zero) { CloseHandle(proc); return null; }
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

    public static int ListCount(IntPtr list) { return (int)Msg(list, 0x1004 /* LVM_GETITEMCOUNT */, 0, 0); }
    public static int ListSelected(IntPtr list) {
        // (A list that does not answer reads as "nothing selected".)
        IntPtr r;
        if (SendMessageTimeout(list, 0x100C /* LVM_GETNEXTITEM */, (IntPtr)(-1), (IntPtr)2 /* LVNI_SELECTED */, 2, 5000, out r) == IntPtr.Zero) return -1;
        return (int)r.ToInt64();
    }

    // Row `item`, column `sub` of a list view in another 64-bit process.
    public static string ListText(IntPtr list, int item, int sub) {
        string s = Remote(list, (proc, mem, h) => {
            var lv = new byte[88];
            BitConverter.GetBytes(1).CopyTo(lv, 0);                         // mask = LVIF_TEXT
            BitConverter.GetBytes(item).CopyTo(lv, 4);                      // iItem
            BitConverter.GetBytes(sub).CopyTo(lv, 8);                       // iSubItem
            BitConverter.GetBytes(mem.ToInt64() + 1024).CopyTo(lv, 24);     // pszText
            BitConverter.GetBytes(2048).CopyTo(lv, 32);                     // cchTextMax
            IntPtr w; WriteProcessMemory(proc, mem, lv, (IntPtr)lv.Length, out w);
            Send(h, 0x1073 /* LVM_GETITEMTEXTW */, (IntPtr)item, mem);
            return ReadString(proc, mem + 1024, 4096);
        });
        return s ?? "";
    }

    public static string StatusText(IntPtr bar, int part) {
        string s = Remote(bar, (proc, mem, h) => {
            Send(h, 0x040D /* SB_GETTEXTW */, (IntPtr)part, mem);
            return ReadString(proc, mem, 4096);
        });
        return s ?? "";
    }

    // Column header `col` of a list view on screen: x, y, w, h (or null).
    public static int[] HeaderRect(IntPtr list, int col) {
        IntPtr header = Send(list, 0x101F /* LVM_GETHEADER */, IntPtr.Zero, IntPtr.Zero);
        if (header == IntPtr.Zero) return null;
        int[] rc = null;
        Remote(header, (proc, mem, h) => {
            if (Send(h, 0x1207 /* HDM_GETITEMRECT */, (IntPtr)col, mem) == IntPtr.Zero) return "";
            var buf = new byte[16]; IntPtr read;
            ReadProcessMemory(proc, mem, buf, (IntPtr)16, out read);
            rc = new[] { BitConverter.ToInt32(buf, 0), BitConverter.ToInt32(buf, 4), BitConverter.ToInt32(buf, 8), BitConverter.ToInt32(buf, 12) };
            return "";
        });
        if (rc == null) return null;
        // Both corners: the header of a right-to-left list is mirrored.
        var a = new POINT(); a.X = rc[0]; a.Y = rc[1];
        var b = new POINT(); b.X = rc[2]; b.Y = rc[3];
        ClientToScreen(header, ref a);
        ClientToScreen(header, ref b);
        return new[] { Math.Min(a.X, b.X), Math.Min(a.Y, b.Y), Math.Abs(b.X - a.X), Math.Abs(b.Y - a.Y) };
    }

    // Chooses entry `index` of a combo box and tells its parent, as a pick
    // with the mouse does (CBN_SELCHANGE).
    public static void ComboPick(IntPtr combo, int index) {
        Send(combo, 0x014E /* CB_SETCURSEL */, (IntPtr)index, IntPtr.Zero);
        IntPtr parent = GetParent(combo);
        int id = GetDlgCtrlID(combo);
        PostMessage(parent, 0x0111 /* WM_COMMAND */, (IntPtr)((1 << 16) | (id & 0xFFFF)), combo);
    }

    // ------------------------------------------------------------ processes
    // Processes that descend from any of `roots`; `name` (an exe name, any
    // case) keeps only those, null keeps all.
    public static int[] Descendants(int[] roots, string name) {
        var found = new List<int>();
        if (roots == null || roots.Length == 0) return found.ToArray();
        var parent = new Dictionary<int, int>();
        var exe = new Dictionary<int, string>();
        IntPtr snap = CreateToolhelp32Snapshot(2 /* TH32CS_SNAPPROCESS */, 0);
        if (snap == IntPtr.Zero || snap == (IntPtr)(-1)) return found.ToArray();
        try {
            var e = new PROCESSENTRY32();
            e.dwSize = (uint)Marshal.SizeOf(typeof(PROCESSENTRY32));
            if (Process32First(snap, ref e)) {
                do {
                    parent[(int)e.th32ProcessID] = (int)e.th32ParentProcessID;
                    exe[(int)e.th32ProcessID] = e.szExeFile ?? "";
                    e.dwSize = (uint)Marshal.SizeOf(typeof(PROCESSENTRY32));
                } while (Process32Next(snap, ref e));
            }
        } finally { CloseHandle(snap); }
        var family = new Dictionary<int, bool>();
        foreach (int r in roots) family[r] = true;
        bool grew = true;
        while (grew) {
            grew = false;
            foreach (var kv in parent) {
                if (!family.ContainsKey(kv.Key) && family.ContainsKey(kv.Value) && kv.Key != 0) { family[kv.Key] = true; grew = true; break; }
            }
        }
        foreach (var kv in family) {
            if (Array.IndexOf(roots, kv.Key) >= 0) continue;
            string n;
            if (!exe.TryGetValue(kv.Key, out n)) continue;
            if (name == null || string.Equals(n, name, StringComparison.OrdinalIgnoreCase)) found.Add(kv.Key);
        }
        return found.ToArray();
    }

    // ------------------------------------------------------------ memory sampler
    // Every `ms`: one CSV row per bahth process and per WebView2 process that
    // descends from one. Runs on a timer thread of its own.
    public static volatile string Phase = "start";
    static Timer sampler;
    static string csvPath;
    static DateTime started;
    static readonly object gate = new object();
    public static int Samples;

    public static void StartSampler(string path, int ms) {
        csvPath = path;
        started = DateTime.UtcNow;
        File.WriteAllText(path, "utc,elapsed_s,phase,process,pid,private_bytes,working_set,threads,handles,gdi_objects,user_objects,free_disk_gb\r\n");
        sampler = new Timer(Sample, null, 0, ms);
    }

    public static void StopSampler() {
        Timer t = sampler; sampler = null;
        if (t != null) t.Dispose();
    }

    static void Row(StringBuilder sb, DateTime now, Process p, string name, double freeGb) {
        long priv = 0, ws = 0; int threads = 0, handles = 0; long gdi = -1, user = -1;
        try { p.Refresh(); priv = p.PrivateMemorySize64; ws = p.WorkingSet64; threads = p.Threads.Count; handles = p.HandleCount; } catch (Exception) { return; }
        try { IntPtr h = p.Handle; gdi = GetGuiResources(h, 0); user = GetGuiResources(h, 1); } catch (Exception) { }
        string phase = (Phase ?? "").Replace(",", ";").Replace("\r", " ").Replace("\n", " ");
        sb.Append(now.ToString("HH:mm:ss")).Append(',').Append((int)(now - started).TotalSeconds).Append(',').Append(phase).Append(',')
          .Append(name).Append(',').Append(p.Id).Append(',').Append(priv).Append(',').Append(ws).Append(',').Append(threads).Append(',')
          .Append(handles).Append(',').Append(gdi).Append(',').Append(user).Append(',').Append(freeGb.ToString("0.0", System.Globalization.CultureInfo.InvariantCulture)).Append("\r\n");
    }

    public static double FreeDiskGb() {
        try { return new DriveInfo(Path.GetPathRoot(Environment.SystemDirectory)).AvailableFreeSpace / 1073741824.0; } catch (Exception) { return -1; }
    }

    static void Sample(object state) {
        if (!Monitor.TryEnter(gate)) return;
        try {
            var sb = new StringBuilder();
            DateTime now = DateTime.UtcNow;
            double free = FreeDiskGb();
            var roots = new List<int>();
            foreach (Process p in Process.GetProcessesByName("bahth")) {
                try { roots.Add(p.Id); Row(sb, now, p, "bahth", free); } catch (Exception) { } finally { p.Dispose(); }
            }
            foreach (int id in Descendants(roots.ToArray(), "msedgewebview2.exe")) {
                try { using (Process p = Process.GetProcessById(id)) Row(sb, now, p, "msedgewebview2", free); } catch (Exception) { }
            }
            if (roots.Count == 0) {
                string phase = (Phase ?? "").Replace(",", ";");
                sb.Append(now.ToString("HH:mm:ss")).Append(',').Append((int)(now - started).TotalSeconds).Append(',').Append(phase).Append(",(none),0,0,0,0,0,0,0,").Append(free.ToString("0.0", System.Globalization.CultureInfo.InvariantCulture)).Append("\r\n");
            }
            File.AppendAllText(csvPath, sb.ToString());
            Samples++;
        } catch (Exception) { } finally { Monitor.Exit(gate); }
    }

    // ------------------------------------------------------------ watchdog
    // The last resort when the script itself is stuck: at the deadline it
    // stops the file churn, ends the program under test, leaves a note (also
    // in the job summary) and ends this PowerShell with exit code 4, so the
    // job never runs into its own timeout.
    static Timer watchdog;
    public static void ArmWatchdog(int seconds, string exePath, string noteFile, string summaryFile) {
        watchdog = new Timer(delegate(object o) {
            try { Churn.RequestStop(); } catch (Exception) { }
            string text = "UI stress watchdog: the test script did not finish within " + (seconds / 60) + " minutes (phase: " + Phase + "). The program under test was ended and the run stopped; see run.log and progress.tsv in the artifact.";
            try { File.AppendAllText(noteFile, DateTime.UtcNow.ToString("s") + " " + text + "\r\n"); } catch (Exception) { }
            try { if (!string.IsNullOrEmpty(summaryFile)) File.AppendAllText(summaryFile, "\n## UI stress: stopped by its watchdog\n\n" + text + "\n"); } catch (Exception) { }
            try {
                foreach (Process p in Process.GetProcessesByName("bahth")) {
                    try { if (string.Equals(p.MainModule.FileName, exePath, StringComparison.OrdinalIgnoreCase)) p.Kill(); } catch (Exception) { }
                }
            } catch (Exception) { }
            Environment.Exit(4);
        }, null, seconds * 1000, Timeout.Infinite);
    }
    public static void DisarmWatchdog() {
        Timer t = watchdog; watchdog = null;
        if (t != null) t.Dispose();
    }
}

// Creates, renames and deletes small files in one folder on a thread of its
// own, so the program's live index has something to keep up with. Bounded:
// at most 300 files alive, about 60 operations a second, and it stops by
// itself when the system drive has less than `minFreeGb` free.
public static class Churn {
    static Thread thread;
    static volatile bool stop;
    public static int Created, Renamed, Deleted, Errors;
    public static string StopReason = "";

    public static void Start(string dir, double minFreeGb, int seed) {
        RequestStop();
        if (thread != null) thread.Join(5000);
        stop = false; Created = 0; Renamed = 0; Deleted = 0; Errors = 0; StopReason = "";
        Directory.CreateDirectory(dir);
        thread = new Thread(delegate() { Run(dir, minFreeGb, seed); });
        thread.IsBackground = true;
        thread.Start();
    }

    public static void RequestStop() { stop = true; }

    public static void Stop() {
        stop = true;
        Thread t = thread;
        if (t != null) t.Join(8000);
        if (StopReason.Length == 0) StopReason = "stopped by the test";
    }

    static void Run(string dir, double minFreeGb, int seed) {
        var rnd = new Random(seed);
        var alive = new List<string>();
        string[] words = { "تقرير", "عقد", "فاتورة", "report", "invoice", "صورة photo", "ملف 2026", "notes" };
        string[] exts = { ".txt", ".txt", ".md", ".csv", ".log" };
        int n = 0;
        while (!stop) {
            try {
                if (n % 50 == 0) {
                    double free = S.FreeDiskGb();
                    if (free >= 0 && free < minFreeGb) { StopReason = "less than " + minFreeGb + " GB free"; break; }
                }
                int op = rnd.Next(10);
                if (alive.Count < 20 || (op < 5 && alive.Count < 300)) {
                    string path = Path.Combine(dir, "zqchurn " + words[rnd.Next(words.Length)] + " " + n + exts[rnd.Next(exts.Length)]);
                    File.WriteAllText(path, "churn " + n);
                    alive.Add(path);
                    Created++;
                } else if (op < 8) {
                    int i = rnd.Next(alive.Count);
                    string to = Path.Combine(dir, "zqchurn " + words[rnd.Next(words.Length)] + " r" + n + exts[rnd.Next(exts.Length)]);
                    File.Move(alive[i], to);
                    alive[i] = to;
                    Renamed++;
                } else {
                    int i = rnd.Next(alive.Count);
                    File.Delete(alive[i]);
                    alive.RemoveAt(i);
                    Deleted++;
                }
            } catch (Exception) { Errors++; }
            n++;
            Thread.Sleep(15);
        }
    }
}
'@
# Real pixels, whatever the display scale.
[void][S]::SetProcessDPIAware()

# ---------------------------------------------------------------- constants
$VK = @{ CTRL = 0x11; ALT = 0x12; SHIFT = 0x10; ENTER = 0x0D; ESC = 0x1B; TAB = 0x09; SPACE = 0x20; BACK = 0x08; DEL = 0x2E; HOME = 0x24; END = 0x23
    UP = 0x26; DOWN = 0x28; RIGHT = 0x27; A = 0x41; C = 0x43; F = 0x46; K = 0x4B; V = 0x56; F2 = 0x71; F4 = 0x73; F10 = 0x79; COMMA = 0xBC }
$WM_CLOSE = 0x0010; $WM_CANCELMODE = 0x001F; $WM_COMMAND = 0x0111; $WM_SYSCOMMAND = 0x0112
$BM_CLICK = 0x00F5; $BM_GETCHECK = 0x00F0
$SC_CLOSE = 0xF060; $SC_MINIMIZE = 0xF020; $SC_MAXIMIZE = 0xF030; $SC_RESTORE = 0xF120
# The tray icon's callback message of native-windows-gui (WM_USER + 102), and
# the right-button-up Explorer sends with it for a right-click on the icon.
$NWG_TRAY = 0x0466; $WM_RBUTTONUP = 0x0205

# The program's own words, in both interface languages (copied from src/ui).
$rx = @{
    Title     = '^(InstaFind|بحث سريع)( \((new|classic)\))?$'
    Options   = '^Options|^خيارات'
    Content   = 'inside documents|داخل المستندات'
    Stop      = 'Stop search|إيقاف البحث'
    TryNew    = 'Try the new look|جرّب الشكل الجديد'
    # Gone in 1.0 (the new look is the only one): only checked to be absent.
    Classic   = 'Back to the classic look|العودة إلى الشكل القديم'
    Hide      = 'Hide to the taskbar tray|إخفاء إلى شريط المهام'
    Digits    = 'Arabic-Indic digits|عرض الأرقام بالعربية'
    Language  = '^Language'
    Anywhere  = 'Search from anywhere|البحث من أي مكان'
    TraySet   = '^Settings…|^الإعدادات…'
    TrayFull  = '^Full window|^النافذة الكاملة'
    TrayExit  = '^Exit$|^خروج$'
}
# Options items that are switches: each is flipped and flipped back.
$rxToggles = @(
    'Arabic-Indic digits|عرض الأرقام بالعربية',
    'Update the index automatically|تحديث الفهرس تلقائياً',
    'Index document contents|فهرسة محتوى المستندات',
    'Hide system files|إخفاء ملفات النظام',
    'Start with Windows|التشغيل مع ويندوز',
    'Jump to the result|القفز إلى مجلد النتيجة',
    'Thumbnail view|عرض المصغرات'
)
# Options items that open a window: each is opened and closed.
$rxDialogs = @(
    'Find duplicate files|البحث عن الملفات المكررة',
    'Folder sizes|أحجام المجلدات',
    'Replace text in files|استبدال نص في الملفات',
    'Search from phone|البحث من الهاتف'
)

$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$crash = Join-Path $data 'crash.txt'
$index = Join-Path $data 'index.bin'
$settingsFile = Join-Path $data 'settings.txt'
$logFile = Join-Path $data 'log.txt'
$rowsFile = Join-Path $data 'quick-rows.txt'
$wvDir = Join-Path $data 'WebView2'
$regKey = 'HKCU:\Software\BahthSari'
$scratch = Join-Path $env:USERPROFILE 'Documents\InstaFind UI stress'
$churnDir = Join-Path $scratch 'churn'
$runLog = Join-Path $Out 'run.log'
$progress = Join-Path $Out 'progress.tsv'
$utf8 = New-Object Text.UTF8Encoding $false

$results = New-Object System.Collections.Generic.List[object]
$memory = New-Object System.Collections.Generic.List[object]
$script:t0 = Get-Date
$script:reserve = $IdleMinutes + 2      # minutes the end state needs
$script:shot = 0
$script:shotMax = 140
$script:proc = $null
$script:hClassic = [IntPtr]::Zero
$script:hNew = [IntPtr]::Zero
$script:expectExit = $false
$script:abort = $false
$script:healthChecks = 0
$script:healthFails = 0
$script:maxPing = 0
$script:revives = 0
$script:indexReady = $false
$script:resultKind = 'probe'            # 'core' once the first index is saved
$script:pageReads = $false              # the page's rows can be read (new look)
$script:lang = 'en'
$script:rnd = New-Object System.Random $Seed
$script:baseWindows = @()
$script:baseNotepads = @()
$script:aclDenied = $false
$script:optionsFails = 0                # tries in a row that opened no Options menu
$script:trayFails = 0                   # the same for the tray menu
$script:optionsClickFails = 0           # real clicks on the Options button that opened no menu

# ---------------------------------------------------------------- report
function Log([string]$Text) {
    try { [IO.File]::AppendAllText($runLog, ("[{0:HH:mm:ss}] {1}`r`n" -f (Get-Date), $Text), $utf8) } catch { }
}
function Say([string]$Text) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $Text); Log $Text }
function Flat([string]$Text, [int]$Max = 400) {
    if ($null -eq $Text) { return '' }
    $t = ($Text -replace '[\r\n\t]+', ' ' -replace '\s{2,}', ' ').Trim()
    if ($t.Length -gt $Max) { $t = $t.Substring(0, $Max) + '…' }
    return $t
}
function Record([string]$Area, [string]$Kind, [string]$Check, [bool]$Ok, [string]$Detail = '') {
    $res = if ($Ok) { 'PASS' } elseif ($Kind -eq 'core') { 'FAIL' } else { 'MISS' }
    $Detail = Flat $Detail 600
    $results.Add([pscustomobject]@{ Area = $Area; Kind = $Kind; Check = $Check; Result = $res; Detail = $Detail; Minute = [Math]::Round(((Get-Date) - $script:t0).TotalMinutes, 1) })
    $line = "[$res] $Area / $Check" + $(if ($Detail) { " - $Detail" } else { '' })
    try { [IO.File]::AppendAllText($progress, "$res`t$Kind`t$Area`t$Check`t$Detail`r`n", $utf8) } catch { }
    # The console stays short: failures and findings; everything is in run.log.
    if ($res -eq 'FAIL') { Write-Host "::error::$line" } elseif ($res -eq 'MISS') { Write-Host "::warning::$line" }
    Log $line
}
function Note([string]$Area, [string]$Check, [string]$Detail) {
    $Detail = Flat $Detail 600
    $results.Add([pscustomobject]@{ Area = $Area; Kind = 'note'; Check = $Check; Result = 'NOTE'; Detail = $Detail; Minute = [Math]::Round(((Get-Date) - $script:t0).TotalMinutes, 1) })
    try { [IO.File]::AppendAllText($progress, "NOTE`tnote`t$Area`t$Check`t$Detail`r`n", $utf8) } catch { }
    Log "[NOTE] $Area / $Check - $Detail"
}
function Shot([string]$Name) {
    if ($script:shot -ge $script:shotMax) { return }
    $script:shot++
    $file = Join-Path $Out ('{0:D3}-{1}.png' -f $script:shot, ($Name -replace '[^A-Za-z0-9]+', '-').Trim('-'))
    try {
        $b = [Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
        $bmp.Save($file, [Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    } catch { Log "screenshot $Name failed: $_" }
}
# How varied the window's client area is: distinct colours in a grid of
# samples and the share of the most common one (a page that never painted
# is one flat colour). Copied from ci/newlook-test.ps1.
function Painted($W) {
    try {
        $c = [S]::ClientOnScreen($W.Handle)
        if ($c[2] -lt 50 -or $c[3] -lt 50) { return $null }
        $bmp = New-Object Drawing.Bitmap $c[2], $c[3]
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($c[0], $c[1], 0, 0, $bmp.Size)
        $counts = @{}
        $n = 0
        for ($y = 2; $y -lt $c[3]; $y += 11) {
            for ($x = 2; $x -lt $c[2]; $x += 11) {
                $k = $bmp.GetPixel($x, $y).ToArgb()
                $counts[$k] = 1 + [int]$counts[$k]
                $n++
            }
        }
        $g.Dispose(); $bmp.Dispose()
        $top = ($counts.Values | Measure-Object -Maximum).Maximum
        return [pscustomobject]@{ Colours = $counts.Count; Top = [Math]::Round($top / [Math]::Max(1, $n), 3) }
    } catch { Log "painted check failed: $_"; return $null }
}
function Elapsed-Min { ((Get-Date) - $script:t0).TotalMinutes }
function Have-Time([double]$Need) { ((Elapsed-Min) + $Need + $script:reserve) -le $MaxMinutes }
function Wait-Until([scriptblock]$Cond, [double]$Seconds = 10, [int]$Ms = 200) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ($true) {
        $v = & $Cond
        if ($v) { return $v }
        if ((Get-Date) -ge $end) { return $null }
        Start-Sleep -Milliseconds $Ms
    }
}
function Now-Unix { [long]([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()) }
# Runs a console program without PowerShell 5.1 turning its stderr into errors.
function Run-Native([string]$File, [string]$Arguments) {
    $o = Join-Path $Out 'native-out.tmp'; $e = Join-Path $Out 'native-err.tmp'
    $p = Start-Process -FilePath $File -ArgumentList $Arguments -NoNewWindow -Wait -PassThru -RedirectStandardOutput $o -RedirectStandardError $e
    $text = ''
    foreach ($f in @($o, $e)) { if (Test-Path $f) { $text += [IO.File]::ReadAllText($f); Remove-Item $f -Force -ErrorAction SilentlyContinue } }
    Log "$File $Arguments -> exit $($p.ExitCode): $(Flat $text 300)"
    return $p.ExitCode
}

# ---------------------------------------------------------------- windows
function Parse-Win([string]$Line) {
    $p = $Line.Split('|', 8)
    [pscustomobject]@{ Handle = [IntPtr][long]$p[0]; Class = $p[1]; Visible = $p[2] -eq 'True'; X = [int]$p[3]; Y = [int]$p[4]; W = [int]$p[5]; H = [int]$p[6]; Text = $p[7] }
}
function Get-Windows([int]$ProcId) { foreach ($l in [S]::Windows($ProcId)) { Parse-Win $l } }
function Get-Children($Hwnd) { foreach ($l in [S]::Children($Hwnd)) { Parse-Win $l } }
function Win-Of($Hwnd) { if ($Hwnd -and $Hwnd -ne [IntPtr]::Zero -and [S]::IsWindow($Hwnd)) { Parse-Win ([S]::Describe($Hwnd)) } }
function App-Windows { if ($script:proc) { Get-Windows $script:proc.Id } }
function Visible-Window([string]$Class) { App-Windows | Where-Object { $_.Visible -and $_.Class -eq $Class } | Select-Object -First 1 }

# The classic window (shown or hidden) holds the Options button; the new
# look's window has the program's title and no such button.
function Handle-Ok($Hwnd) { $Hwnd -ne [IntPtr]::Zero -and [S]::IsWindow($Hwnd) -and $script:proc -and ([S]::Pid($Hwnd) -eq $script:proc.Id) }
function Classic-Handle {
    if (Handle-Ok $script:hClassic) { return $script:hClassic }
    $script:hClassic = [IntPtr]::Zero
    foreach ($w in @(App-Windows | Where-Object { $_.Class -eq 'NativeWindowsGuiWindow' })) {
        if (Get-Children $w.Handle | Where-Object { $_.Class -eq 'Button' -and $_.Text -match $rx.Options } | Select-Object -First 1) { $script:hClassic = $w.Handle; break }
    }
    return $script:hClassic
}
function New-Handle {
    if (Handle-Ok $script:hNew) { return $script:hNew }
    $script:hNew = [IntPtr]::Zero
    $c = Classic-Handle
    foreach ($w in @(App-Windows | Where-Object { $_.Class -eq 'NativeWindowsGuiWindow' })) {
        if ($w.Handle -ne $c -and $w.Text -match $rx.Title) { $script:hNew = $w.Handle; break }
    }
    return $script:hNew
}
# Which main window is on screen: classic, new, none (in the tray) or both.
function Look-Now {
    $c = Classic-Handle; $n = New-Handle
    $cv = ($c -ne [IntPtr]::Zero) -and [S]::IsWindowVisible($c)
    $nv = ($n -ne [IntPtr]::Zero) -and [S]::IsWindowVisible($n)
    if ($cv -and $nv) { return 'both' }
    if ($cv) { return 'classic' }
    if ($nv) { return 'new' }
    return 'none'
}
function Shown-Main {
    $l = Look-Now
    if ($l -eq 'classic') { return (Win-Of (Classic-Handle)) }
    if ($l -eq 'new' -or $l -eq 'both') { return (Win-Of (New-Handle)) }
    return $null
}
# Other windows of the program on screen (duplicates, folder sizes...).
function Extra-Windows {
    $c = Classic-Handle; $n = New-Handle
    @(App-Windows | Where-Object { $_.Visible -and $_.Class -eq 'NativeWindowsGuiWindow' -and $_.Handle -ne $c -and $_.Handle -ne $n -and $_.W -gt 150 -and $_.H -gt 100 })
}
function Focus-Window($Hwnd) {
    if ([S]::GetForegroundWindow() -eq $Hwnd) { return }
    # Windows only hands the foreground to the process that last had input;
    # a tap of Alt counts as that. The tap can leave the window in menu mode
    # (a popup menu would close at once): WM_CANCELMODE leaves it.
    [S]::Chord($VK.ALT)
    [void][S]::SetForegroundWindow($Hwnd)
    Start-Sleep -Milliseconds 250
    [void][S]::Post($Hwnd, $WM_CANCELMODE, 0, 0)
    Start-Sleep -Milliseconds 120
}
function Click-Center($W, [bool]$Right = $false) { [S]::Click([int]($W.X + $W.W / 2), [int]($W.Y + $W.H / 2), $Right) }
# The classic window's controls, found by class (and the buttons by text).
function Find-Controls {
    $main = Win-Of (Classic-Handle)
    if (-not $main) { return $null }
    $kids = @(Get-Children $main.Handle | Where-Object Visible)
    $buttons = @($kids | Where-Object Class -eq 'Button')
    [pscustomobject]@{
        Main    = $main
        Query   = $kids | Where-Object Class -eq 'Edit' | Sort-Object Y | Select-Object -First 1
        List    = $kids | Where-Object Class -eq 'SysListView32' | Select-Object -First 1
        Status  = $kids | Where-Object Class -eq 'msctls_statusbar32' | Select-Object -First 1
        Kind    = $kids | Where-Object Class -eq 'ComboBox' | Select-Object -First 1
        Content = $buttons | Where-Object { $_.Text -match $rx.Content } | Select-Object -First 1
        Options = $buttons | Where-Object { $_.Text -match $rx.Options } | Select-Object -First 1
        Stop    = $buttons | Where-Object { $_.Text -match $rx.Stop } | Select-Object -First 1
        All     = $kids
    }
}
function Clean([string]$Text) { if ($null -eq $Text) { return '' }; return ($Text -replace '[\u200E\u200F\u202A-\u202E\u2066-\u2069\u061C]', '') }
function Status-Text {
    $ui = Find-Controls
    if (-not $ui -or -not $ui.Status) { return '' }
    return (Clean (([S]::StatusText($ui.Status.Handle, 0) + ' | ' + [S]::StatusText($ui.Status.Handle, 1) + ' | ' + [S]::StatusText($ui.Status.Handle, 2)).Trim()))
}
function Main-Names([int]$Max = 300) {
    $ui = Find-Controls
    if (-not $ui -or -not $ui.List) { return @() }
    $n = [Math]::Min([S]::ListCount($ui.List.Handle), $Max)
    $names = @()
    for ($i = 0; $i -lt $n; $i++) { $names += (Clean ([S]::ListText($ui.List.Handle, $i, 0))) }
    return $names
}

# ---------------------------------------------------------------- clipboard
function Set-Clip([string]$Text) {
    for ($i = 0; $i -lt 4; $i++) {
        try { Set-Clipboard -Value $Text -ErrorAction Stop; return $true } catch { Start-Sleep -Milliseconds 150 }
    }
    try { [Windows.Forms.Clipboard]::SetText($Text); return $true } catch { Log "clipboard: cannot set ($_)"; return $false }
}
function Get-Clip {
    for ($i = 0; $i -lt 4; $i++) {
        try { $t = Get-Clipboard -Raw -ErrorAction Stop; if ($null -eq $t) { return '' } else { return [string]$t } } catch { Start-Sleep -Milliseconds 150 }
    }
    try { return [Windows.Forms.Clipboard]::GetText() } catch { return $null }
}

# ---------------------------------------------------------------- settings, log
function Read-Settings { if (Test-Path $settingsFile) { try { [IO.File]::ReadAllText($settingsFile) } catch { '' } } else { '' } }
function Setting([string]$Key) {
    $m = [regex]::Matches((Read-Settings), "(?m)^\s*$Key\s*=\s*(.*?)\s*$")
    if ($m.Count) { $m[$m.Count - 1].Groups[1].Value } else { $null }
}
# settings.txt as the program reads it (key=value, ASCII, no BOM), written
# only while the program is not running: no first-run question (moving the
# exe, rating; 1.0 no longer asks about the box above the clock, which is
# off unless turned on, so it is turned on here as a user would), the
# floating box on, 1.0's one-time look markers (new_ui_default=1,
# new_ui_only=1: nothing to move), and the daily update check already
# "done" so the program does not ask GitHub. (new_ui=0 is ignored since
# 1.0: the classic look comes from BAHTH_CLASSIC=1.)
function Write-Settings([int]$NewUi, [int]$NewUiFailed = 0) {
    New-Item -ItemType Directory -Force $data | Out-Null
    $now = Now-Unix
    $text = "floating_asked=1`nmove_asked=1`nrating_asked=1`nfloating_mode=fade`ntray_icon=1`nhotkey_opens=quick`nalt_space=0`nkind=0`nsort_col=-1`nsort_desc=0`nupdate_check=$now`ncrash_seen=$now`nnew_ui=$NewUi`nnew_ui_failed=$NewUiFailed`nnew_ui_default=1`nnew_ui_only=1`n"
    [IO.File]::WriteAllText($settingsFile, $text, (New-Object Text.ASCIIEncoding))
    Log "settings.txt written: $($text -replace "`n", ' ')"
}
# Exchange rates "downloaded just now" (the program's bundled values), so the
# quick search does not ask the rates service during the test.
function Write-Rates {
    $now = Now-Unix
    $rates = '"USD":1.0,"EUR":0.86,"GBP":0.76,"IQD":1310.0,"SAR":3.75,"AED":3.6725,"KWD":0.306,"QAR":3.64,"BHD":0.376,"OMR":0.3845,"JOD":0.709,"EGP":47.5,"TRY":42.5,"IRR":42000.0,"SYP":13000.0,"LBP":89500.0,"JPY":152.0,"CNY":7.12,"INR":88.5,"CAD":1.40,"AUD":1.53,"CHF":0.80,"RUB":81.0'
    $json = '{"version":1,"updated":' + ($now - 3600) + ',"fetched":' + $now + ',"rates":{' + $rates + '}}'
    [IO.File]::WriteAllText((Join-Path $data 'rates.json'), $json, (New-Object Text.ASCIIEncoding))
}
function Log-Text { if (Test-Path $logFile) { try { [IO.File]::ReadAllText($logFile, [Text.Encoding]::UTF8) } catch { '' } } else { '' } }
function Log-Lines([string]$Pattern, [int]$From = 0) {
    $t = Log-Text
    if ($From -gt 0 -and $From -le $t.Length) { $t = $t.Substring($From) }
    @($t -split "`r?`n" | Where-Object { $_ -match $Pattern })
}
# Where the program's log ends now (it is emptied past 64 KB: then 0).
function Log-Mark { (Log-Text).Length }
# The program's log, appended to <Out>\log.txt before anything deletes it.
function Keep-Log([string]$When) {
    try {
        if (Test-Path $logFile) {
            [IO.File]::AppendAllText((Join-Path $Out 'log.txt'), "==== $When ($(Get-Date -Format s))`r`n" + (Log-Text) + "`r`n", $utf8)
        }
    } catch { Log "keeping log.txt failed: $_" }
}

# ---------------------------------------------------------------- the program
function Bahth-Procs {
    @(Get-Process -Name bahth -ErrorAction SilentlyContinue | Where-Object {
            $path = $null
            try { $path = $_.Path } catch { }
            (-not $path) -or ($path -eq $Exe)
        })
}
function WebView-Pids {
    $roots = @(Bahth-Procs | ForEach-Object { $_.Id })
    if (-not $roots.Count) { return @() }
    return @([S]::Descendants([int[]]$roots, 'msedgewebview2.exe'))
}
function Proc-Alive { $script:proc -and ($null -ne (Get-Process -Id $script:proc.Id -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -eq 'bahth' })) }
function Exit-Code { try { return [string]$script:proc.ExitCode } catch { return '?' } }
# Memory of the program and of its WebView2 children, in MB.
function Mem-Now {
    $b = @(Bahth-Procs)
    $priv = 0; $ws = 0; $threads = 0; $handles = 0
    foreach ($p in $b) { try { $p.Refresh(); $priv += $p.PrivateMemorySize64; $ws += $p.WorkingSet64; $threads += $p.Threads.Count; $handles += $p.HandleCount } catch { } }
    $wv = @(WebView-Pids); $wpriv = 0; $wws = 0
    foreach ($id in $wv) { $q = Get-Process -Id $id -ErrorAction SilentlyContinue; if ($q) { $wpriv += $q.PrivateMemorySize64; $wws += $q.WorkingSet64 } }
    [pscustomobject]@{ Procs = $b.Count; PrivateMB = [Math]::Round($priv / 1MB, 1); WorkingMB = [Math]::Round($ws / 1MB, 1); Threads = $threads; Handles = $handles
        WebViews = $wv.Count; WvPrivateMB = [Math]::Round($wpriv / 1MB, 1); WvWorkingMB = [Math]::Round($wws / 1MB, 1) }
}
function Mem-Note([string]$When) {
    $m = Mem-Now
    $memory.Add([pscustomobject]@{ When = $When; Minute = [Math]::Round((Elapsed-Min), 1); M = $m })
    Log ("memory [{0}]: bahth x{1} private {2} MB, working set {3} MB, {4} threads, {5} handles; WebView2 x{6} private {7} MB" -f $When, $m.Procs, $m.PrivateMB, $m.WorkingMB, $m.Threads, $m.Handles, $m.WebViews, $m.WvPrivateMB)
    return $m
}
function Stop-App([string]$Why = '') {
    Log "stopping the program ($Why)"
    $script:expectExit = $true
    $wv = @(WebView-Pids)
    foreach ($p in @(Bahth-Procs)) { try { $p.Kill() } catch { } }
    [void](Wait-Until { -not @(Bahth-Procs).Count } 10)
    # Its WebView2 processes follow it; the ones that do not are ended.
    [void](Wait-Until { -not @($wv | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue | Where-Object ProcessName -eq 'msedgewebview2' }).Count } 8)
    foreach ($id in $wv) { $q = Get-Process -Id $id -ErrorAction SilentlyContinue | Where-Object ProcessName -eq 'msedgewebview2'; if ($q) { try { $q.Kill() } catch { } } }
    $script:proc = $null; $script:hClassic = [IntPtr]::Zero; $script:hNew = [IntPtr]::Zero
    Start-Sleep -Milliseconds 500
}
# The main window at the top left, clear of the taskbar and of the floating
# box above the clock (a click meant for a button must not land on the box).
function Place-Main {
    $m = Shown-Main
    if (-not $m) { return }
    $scr = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    if ([S]::IsZoomed($m.Handle) -or [S]::IsIconic($m.Handle)) { [void][S]::Post($m.Handle, $WM_SYSCOMMAND, $SC_RESTORE, 0); Start-Sleep -Milliseconds 250 }
    [S]::Place($m.Handle, 16, 16, [Math]::Min(1000, $scr.Width - 32), [Math]::Max(420, [Math]::Min(640, $scr.Height - 120)))
    Start-Sleep -Milliseconds 250
}
function Start-App([int]$Wait = 40) {
    $script:hClassic = [IntPtr]::Zero; $script:hNew = [IntPtr]::Zero
    $script:proc = Start-Process -FilePath $Exe -PassThru
    try { [void]$script:proc.Handle } catch { }
    $script:expectExit = $false
    $script:appStart = Get-Date
    $script:optionsFails = 0; $script:trayFails = 0
    Log "started the program, process $($script:proc.Id)"
    $w = Wait-Until { Shown-Main } $Wait
    if ($w) { Place-Main; $w = Shown-Main }
    return $w
}
# After the program restarted itself (language): the new process.
function Adopt-Restarted([int]$OldId, [int]$Wait = 45) {
    $new = Wait-Until { Bahth-Procs | Where-Object { $_.Id -ne $OldId } | Select-Object -First 1 } $Wait
    if (-not $new) { return $null }
    $script:proc = $new
    try { [void]$script:proc.Handle } catch { }
    $script:hClassic = [IntPtr]::Zero; $script:hNew = [IntPtr]::Zero
    $script:expectExit = $false
    $script:appStart = Get-Date
    $script:optionsFails = 0; $script:trayFails = 0
    Log "the program restarted itself: process $($new.Id)"
    $w = Wait-Until { Shown-Main } $Wait
    if ($w) { Place-Main; $w = Shown-Main }
    return $w
}
# A second start of the exe: it finds the running copy, shows its window and exits.
function Second-Launch {
    $p2 = Start-Process -FilePath $Exe -PassThru
    if (-not $p2.WaitForExit(8000)) {
        Record 'second start' 'probe' 'a second start exits after showing the running copy' $false "process $($p2.Id) still runs after 8 s"
        if ($script:proc -and $p2.Id -ne $script:proc.Id -and (Proc-Alive)) { try { $p2.Kill() } catch { } }
        return $false
    }
    return $true
}

# ---------------------------------------------------------------- dialogs, menus
function Dialogs { @(App-Windows | Where-Object { $_.Visible -and $_.Class -eq '#32770' }) }
function Dialog-Text($D) { return (Flat ("$($D.Text): " + ((Get-Children $D.Handle | Where-Object { $_.Class -eq 'Static' -and $_.Text } | ForEach-Object Text) -join ' ')) 300) }
# Answers every message box or file dialog of the program ('no': No, Cancel,
# OK; 'yes': Yes, OK). Returns what they said.
function Dismiss-Dialogs([string]$Answer = 'no', [string]$Why = '') {
    $seen = @()
    for ($round = 0; $round -lt 5; $round++) {
        $ds = @(Dialogs)
        if (-not $ds.Count) { break }
        foreach ($d in $ds) {
            $text = Dialog-Text $d
            if ($round -eq 0) { $seen += $text }
            Log "dialog ($Why): $text -> $Answer (round $round)"
            $ids = if ($Answer -eq 'yes') { @(6, 1, 2) } else { @(7, 2, 1) }
            $done = $false
            foreach ($id in $ids) {
                $b = [S]::DlgItem($d.Handle, $id)
                if ($b -ne 0) {
                    # The button's own click first; the command it sends next.
                    if ($round % 2 -eq 0) { [void][S]::Post([IntPtr][long]$b, $BM_CLICK, 0, 0) } else { [void][S]::Post($d.Handle, $WM_COMMAND, $id, $b) }
                    $done = $true
                    break
                }
            }
            if (-not $done -or $round -ge 3) { [void][S]::Post($d.Handle, $WM_CLOSE, 0, 0) }
        }
        Start-Sleep -Milliseconds 350
    }
    # Windows' own "file operation" window (a delete that cannot be done).
    foreach ($w in @(App-Windows | Where-Object { $_.Visible -and $_.Class -eq 'OperationStatusWindow' })) {
        $seen += "OperationStatusWindow: $($w.Text)"
        [void][S]::Post($w.Handle, $WM_CLOSE, 0, 0)
    }
    return , $seen
}
function Close-Extras {
    $closed = @()
    foreach ($w in @(Extra-Windows)) { $closed += $w.Text; [void][S]::Post($w.Handle, $WM_SYSCOMMAND, $SC_CLOSE, 0) }
    if ($closed.Count) { Start-Sleep -Milliseconds 300 }
    return , $closed
}
function Menu-Windows { @(Get-Windows 0 | Where-Object { $_.Class -eq '#32768' }) }
function Menu-Items($MenuWin) {
    foreach ($l in [S]::MenuDump($MenuWin.Handle)) {
        $p = $l.Split('|', 5)
        $state = [int]$p[1]
        [pscustomobject]@{ Pos = [int]$p[0]; Sub = [long]$p[2]; Menu = [long]$p[3]; Text = $p[4]
            Checked = ($state -band 0x8) -ne 0; Grayed = ($state -band 0x3) -ne 0; Separator = (($state -band 0x800) -ne 0) -and (-not $p[4]) }
    }
}
function Wait-Menu([double]$Seconds = 4) { Wait-Until { Menu-Windows | Select-Object -First 1 } $Seconds 80 }
# Closes every open popup menu: Esc when asked (only while a menu is really
# open: Esc in the main window would hide it), then WM_CANCELMODE.
function Close-Menus([switch]$Escape) {
    for ($i = 0; $i -lt 6; $i++) {
        if (-not @(Menu-Windows).Count) { return }
        if ($Escape -and $i -eq 0) { [S]::Chord($VK.ESC) }
        else {
            $c = Classic-Handle
            if ($c -ne [IntPtr]::Zero) { [void][S]::Post($c, $WM_CANCELMODE, 0, 0) }
            $n = New-Handle
            if ($n -ne [IntPtr]::Zero) { [void][S]::Post($n, $WM_CANCELMODE, 0, 0) }
        }
        Start-Sleep -Milliseconds 200
    }
}
# Chooses an item of an open popup menu with the mouse (its rectangle from
# GetMenuItemRect); with the keyboard when the rectangle cannot be had.
function Click-MenuItem($MenuWin, $Item) {
    $r = [S]::MenuItemRect($MenuWin.Handle, $Item.Pos)
    if ($r -and $r[2] -gt 6 -and $r[3] -gt 4) {
        [S]::Click([int]($r[0] + $r[2] / 2), [int]($r[1] + $r[3] / 2), $false)
        return 'mouse'
    }
    $downs = @(Menu-Items $MenuWin | Where-Object { $_.Pos -le $Item.Pos -and -not $_.Separator }).Count
    [S]::Tap($VK.DOWN, $downs, 40)
    [S]::Chord($VK.ENTER)
    return 'keyboard'
}
# The tray icon's menu. A right-click on the icon reaches the program as the
# icon's callback message; that message is posted here (the icon itself may
# sit in the taskbar's overflow, where the mouse cannot reach it).
function Open-TrayMenu {
    # (After three tries in a row without a menu, no more time is spent on it
    # until the program is started again.)
    if ($script:trayFails -ge 3) { return $null }
    Close-Menus
    $h = Classic-Handle
    if ($h -eq [IntPtr]::Zero) { return $null }
    [S]::MoveCursor(320, 190)
    [void][S]::Post($h, $NWG_TRAY, 0, $WM_RBUTTONUP)
    $m = Wait-Until { Menu-Windows | Where-Object { Menu-Items $_ | Where-Object { $_.Text -match $rx.TrayExit } } | Select-Object -First 1 } 4 80
    if ($m) { $script:trayFails = 0 }
    else {
        $script:trayFails++
        if ($script:trayFails -eq 3) {
            Shot 'tray-menu-does-not-open'
            Record 'menus' 'probe' 'SCRIPT: the tray menu opens when its right-click message is posted' $false "three tries in a row opened no menu; steps that need it are skipped until the next start. Menus on screen: $((Menu-Windows | ForEach-Object { (Menu-Items $_ | ForEach-Object Text) -join ' / ' }) -join ' ;; ')"
        }
    }
    return $m
}
function Tray-Choose([string]$Pattern) {
    $t = Open-TrayMenu
    if (-not $t) { Log "tray menu did not open (wanted $Pattern)"; return $false }
    $it = Menu-Items $t | Where-Object { $_.Text -match $Pattern } | Select-Object -First 1
    if (-not $it) { Log ("tray menu has no item ${Pattern}: " + ((Menu-Items $t | ForEach-Object Text) -join ' / ')); Close-Menus; return $false }
    [void](Click-MenuItem $t $it)
    return $true
}
# The Options menu: the button in the classic window; from the tray menu's
# "Settings…" when the new look (or nothing) is on screen.
function Open-Options {
    if ($script:optionsFails -ge 3) { return $null }
    $m = Open-Options-Once
    if ($m) { $script:optionsFails = 0 }
    else {
        $script:optionsFails++
        if ($script:optionsFails -eq 3) {
            Shot 'options-menu-does-not-open'
            Record 'menus' 'probe' 'SCRIPT: the Options menu can be opened by the test' $false "three tries in a row opened no menu (on screen: $(Look-Now)); steps that need it are skipped until the next start"
        }
    }
    return $m
}
function Open-Options-Once {
    Close-Menus
    $isOptions = { Menu-Windows | Where-Object { Menu-Items $_ | Where-Object { $_.Text -match $rx.Digits } } | Select-Object -First 1 }
    if ((Look-Now) -eq 'classic') {
        $ui = Find-Controls
        if ($ui -and $ui.Options) {
            Focus-Window $ui.Main.Handle
            [void][S]::Post($ui.Main.Handle, $WM_CANCELMODE, 0, 0)
            Start-Sleep -Milliseconds 60
            # A real click on the button while it is on the screen (twice
            # without a menu: not tried again); else the click as a message.
            $scr = [Windows.Forms.SystemInformation]::VirtualScreen
            $cx = [int]($ui.Options.X + $ui.Options.W / 2); $cy = [int]($ui.Options.Y + $ui.Options.H / 2)
            if ($script:optionsClickFails -lt 2 -and $cx -gt 4 -and $cy -gt 4 -and $cx -lt ($scr.Width - 4) -and $cy -lt ($scr.Height - 4) -and -not [S]::IsIconic($ui.Main.Handle)) {
                [S]::Click($cx, $cy, $false)
                $m = Wait-Until $isOptions 2 80
                if ($m) { $script:optionsClickFails = 0; return $m }
                $script:optionsClickFails++
                Log "Options button: a real click at $cx,$cy opened no menu; posting the click instead"
                Close-Menus
                [void](Close-Popup)
            }
            [S]::MoveCursor(300, 160)
            Start-Sleep -Milliseconds 80
            [void][S]::Post($ui.Options.Handle, $BM_CLICK, 0, 0)
            $m = Wait-Until $isOptions 3 80
            if ($m) { return $m }
            Log 'Options button: no menu; trying the tray menu'
        }
    }
    if (-not (Tray-Choose $rx.TraySet)) { return $null }
    return (Wait-Until $isOptions 4 80)
}
# Opens the Options menu and chooses the item matching $Pattern (in the
# submenu matching $Parent when given). Returns the item as it was (with its
# check mark), or $null.
function Options-Choose([string]$Pattern, [string]$Parent = '', [int]$SubPos = -1) {
    $m = Open-Options
    if (-not $m) { Log "Options menu did not open (wanted $Pattern)"; return $null }
    $items = @(Menu-Items $m)
    if ($Parent) {
        $p = $items | Where-Object { $_.Sub -ne 0 -and $_.Text -match $Parent } | Select-Object -First 1
        if (-not $p) { Log "Options has no submenu $Parent"; Close-Menus; return $null }
        [void](Click-MenuItem $m $p)
        $wantMenu = $p.Sub; $firstMenu = $m.Handle
        $sub = Wait-Until { Menu-Windows | Where-Object { $_.Handle -ne $firstMenu -and (@(Menu-Items $_ | Where-Object { $_.Menu -eq $wantMenu }).Count -gt 0) } | Select-Object -First 1 } 3 80
        if (-not $sub) { Log "submenu $Parent did not open"; Close-Menus; return $null }
        $subItems = @(Menu-Items $sub)
        $it = if ($SubPos -ge 0) { $subItems | Where-Object { $_.Pos -eq $SubPos } | Select-Object -First 1 } else { $subItems | Where-Object { $_.Text -match $Pattern } | Select-Object -First 1 }
        if (-not $it) { Log "submenu $Parent has no item $Pattern/$SubPos"; Close-Menus; return $null }
        [void](Click-MenuItem $sub $it)
        Start-Sleep -Milliseconds 250
        Close-Menus
        return $it
    }
    $it = $items | Where-Object { $_.Sub -eq 0 -and $_.Text -match $Pattern } | Select-Object -First 1
    if (-not $it) { Log "Options has no item $Pattern"; Close-Menus; return $null }
    [void](Click-MenuItem $m $it)
    Start-Sleep -Milliseconds 250
    return $it
}
# Every item of the Options menu with its check mark, to compare before and after.
function Options-State {
    $m = Open-Options
    if (-not $m) { return $null }
    $lines = @(Menu-Items $m | Where-Object { -not $_.Separator } | ForEach-Object { ('{0}{1}' -f $(if ($_.Checked) { '[x] ' } else { '[ ] ' }), $_.Text) })
    Close-Menus
    return $lines
}
# The quick search popup (Ctrl+Alt+F).
function Popup { Visible-Window 'InstaFindQuickSearch' }
function Close-Popup {
    for ($i = 0; $i -lt 3; $i++) {
        $p = Popup
        if (-not $p) { return $true }
        if ([S]::GetForegroundWindow() -ne $p.Handle) { Focus-Window $p.Handle }
        [S]::Chord($VK.ESC)
        if (Wait-Until { -not (Popup) } 1.5 80) { return $true }
    }
    return (-not (Popup))
}
# Windows the test's own actions opened (an editor for a result, an Explorer
# folder): closed politely; only editors (Notepad) that were not running
# before the test are ended. Nothing else is ever touched.
function Close-Opened([string]$Why) {
    $mine = @(Bahth-Procs | ForEach-Object { $_.Id }) + @(WebView-Pids) + @($PID)
    $closed = 0
    foreach ($w in @(Get-Windows 0)) {
        if ($script:baseWindows -contains $w.Handle.ToInt64()) { continue }
        $owner = [S]::Pid($w.Handle)
        if ($mine -contains $owner) { continue }
        $name = ''
        try { $name = (Get-Process -Id $owner -ErrorAction Stop).ProcessName } catch { }
        if ($name -match '^(notepad|explorer)$' -and $w.Class -match '^(Notepad|CabinetWClass|ApplicationFrameWindow)$') {
            [void][S]::Post($w.Handle, $WM_CLOSE, 0, 0)
            $closed++
        } elseif ($w.Class -notmatch '^(#32768|tooltips_class32|IME|MSCTFIME UI|Shell_TrayWnd|SysShadow|NotifyIconOverflowWindow|TopLevelWindowForOverflowXamlIsland|Xaml_WindowedPopupClass|TaskListThumbnailWnd)$') {
            Log "new window left alone ($Why): $name / $($w.Class) / $($w.Text)"
        }
    }
    Start-Sleep -Milliseconds 400
    $pads = @(Get-Process -Name notepad -ErrorAction SilentlyContinue | Where-Object { $script:baseNotepads -notcontains $_.Id })
    foreach ($p in $pads) { try { $p.Kill() } catch { } }
    if ($closed -or $pads.Count) { Log "closed $closed window(s), ended $($pads.Count) Notepad process(es) the test opened ($Why)" }
    return ($closed + $pads.Count)
}

# ---------------------------------------------------------------- health
function Revive([string]$Why) {
    $script:revives++
    Keep-Log "before restart: $Why"
    if ($script:revives -gt 6) { $script:abort = $true; Say 'The program had to be restarted too often: the remaining scenarios are skipped.'; return }
    Say "restarting the program after: $Why"
    Stop-App "revive: $Why"
    $w = Start-App 60
    if (-not $w) {
        Record 'stability' 'core' "program starts again after: $Why" $false 'no main window within 60 s'
        $script:abort = $true
    }
}
# After every step: no crash report, the process is alive, the window's
# thread answers within 10 s, no message box left open.
function Health([string]$When, [switch]$Quick) {
    $script:healthChecks++
    if (Test-Path -LiteralPath $crash) {
        $script:healthFails++
        $text = ''
        try { $text = [IO.File]::ReadAllText($crash, [Text.Encoding]::UTF8) } catch { $text = "unreadable: $_" }
        try { [IO.File]::AppendAllText((Join-Path $Out 'crash.txt'), "==== after: $When ($(Get-Date -Format s))`r`n$text`r`n", $utf8) } catch { }
        Remove-Item -LiteralPath $crash -Force -ErrorAction SilentlyContinue
        Record 'stability' 'core' "no crash report ($When)" $false $text
        Shot "crash-$When"
    }
    if ($script:proc -and -not $script:expectExit -and -not (Proc-Alive)) {
        $script:healthFails++
        Record 'stability' 'core' "program still running ($When)" $false "process $($script:proc.Id) is gone, exit code $(Exit-Code); last log lines: $((Log-Lines '.' | Select-Object -Last 4) -join ' // ')"
        Shot "exited-$When"
        Revive "exit during $When"
        return
    }
    if ($Quick -or -not $script:proc -or $script:expectExit) { return }
    $h = Classic-Handle
    if ($h -eq [IntPtr]::Zero) { return }
    $ms = [S]::Ping($h, 10000)
    if ($ms -ge 0) {
        if ($ms -gt $script:maxPing) { $script:maxPing = $ms }
        if ($ms -gt 3000) { Note 'stability' "slow answer ($When)" "the window took $ms ms to answer" }
    } elseif ($ms -eq -1) {
        $script:healthFails++
        Shot "hang-$When"
        $hung = [S]::IsHungAppWindow($h)
        # How long does it last? Up to a minute more.
        $t = Get-Date
        $back = Wait-Until { ([S]::Ping($h, 5000)) -ge 0 } 50 500
        $total = 10 + ((Get-Date) - $t).TotalSeconds
        if ($back) {
            Record 'stability' 'core' "window answers within 10 s ($When)" $false ("hung for about {0:N0} s, then answered (IsHungAppWindow: {1})" -f $total, $hung)
        } else {
            Record 'stability' 'core' "window answers within 10 s ($When)" $false ("still hung after {0:N0} s (IsHungAppWindow: {1}); the program is ended and started again" -f $total, $hung)
            Revive "hang during $When"
            return
        }
    } else {
        Log "ping ($When): $ms"
    }
    $left = Dismiss-Dialogs 'no' "left open after $When"
    if ($left.Count) { Note 'dialogs' "message left open ($When)" ($left -join ' // ') }
}
# Runs one scenario if the time budget still has room for it; a crash check
# follows whatever happened inside.
function Scenario([string]$Name, [double]$Need, [scriptblock]$Body) {
    if ($script:abort) { Note 'budget' $Name 'skipped: the run was aborted'; return }
    if (-not (Have-Time $Need)) {
        Note 'budget' $Name ("skipped: needs about {0} min, {1:N1} of {2} used, {3} kept for the end state" -f $Need, (Elapsed-Min), $MaxMinutes, $script:reserve)
        Say "-- skipped (time budget): $Name"
        return
    }
    Say ("== {0} (minute {1:N1})" -f $Name, (Elapsed-Min))
    [S]::Phase = $Name
    $t = Get-Date
    try { & $Body }
    catch {
        Record $Name 'probe' 'SCRIPT: the scenario ran to its end' $false "test script error: $($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
        Shot "script-error-$Name"
    }
    try {
        Close-Menus
        [void](Close-Popup)
        Health $Name
        [void](Close-Extras)
    } catch { Log "after ${Name}: $_" }
    Log ("scenario '{0}' took {1:N0} s" -f $Name, ((Get-Date) - $t).TotalSeconds)
}

# ---------------------------------------------------------------- the search box
# Puts the main window in front (out of the tray if need be) and the caret
# in its search box.
function Restore-Main([string]$How = 'launch') {
    $m = Shown-Main
    if ($m) {
        if ([S]::IsIconic($m.Handle)) { [void][S]::Post($m.Handle, $WM_SYSCOMMAND, $SC_RESTORE, 0); Start-Sleep -Milliseconds 300; $m = Shown-Main }
        return $m
    }
    if (-not (Proc-Alive)) { return $null }
    if ($How -eq 'tray') { [void](Tray-Choose $rx.TrayFull) } else { [void](Second-Launch) }
    $m = Wait-Until { Shown-Main } 8
    if (-not $m -and $How -eq 'tray') { [void](Second-Launch); $m = Wait-Until { Shown-Main } 8 }
    return $m
}
function Focus-Query {
    $m = Restore-Main
    if (-not $m) { return $false }
    Focus-Window $m.Handle
    if ((Look-Now) -eq 'classic') {
        $ui = Find-Controls
        if (-not $ui -or -not $ui.Query) { return $false }
        Click-Center $ui.Query
        Start-Sleep -Milliseconds 120
    } else {
        # A click on the search field at the top of the page gives the page
        # the keyboard; Ctrl+K then puts the caret in the box, all selected.
        $c = [S]::ClientOnScreen($m.Handle)
        [S]::Click([int]($c[0] + $c[2] / 2), $c[1] + 34, $false)
        Start-Sleep -Milliseconds 150
        [S]::Chord($VK.CTRL, $VK.K)
        Start-Sleep -Milliseconds 120
    }
    return $true
}
# Selects what is in the box (so typing replaces it).
function Select-Query { if ((Look-Now) -eq 'classic') { [S]::Chord($VK.CTRL, $VK.A) } else { [S]::Chord($VK.CTRL, $VK.K) }; Start-Sleep -Milliseconds 30 }
function Clear-Query { Select-Query; [S]::Chord($VK.DEL); Start-Sleep -Milliseconds 60 }
function Type-Query([string]$Text, [int]$Chunk = 6, [int]$Pause = 6) { Select-Query; [S]::Type($Text, $Chunk, $Pause) }
# What the search box holds: WM_GETTEXT in the classic window; select all and
# copy in the new look.
function Read-Query {
    if ((Look-Now) -eq 'classic') {
        $ui = Find-Controls
        if ($ui -and $ui.Query) { return [S]::Text($ui.Query.Handle) }
        return $null
    }
    $mark = "zq-none-$($script:rnd.Next())"
    [void](Set-Clip $mark)
    [S]::Chord($VK.CTRL, $VK.A); Start-Sleep -Milliseconds 60
    [S]::Chord($VK.CTRL, $VK.C)
    $t = Wait-Until { $c = Get-Clip; if ($c -and $c -ne $mark) { $c } } 1.5 100
    if (-not $t) { return $null }
    return [string]$t
}
# New look: the paths of the first rows, read with the page's own "copy
# path" key (Ctrl+Shift+C) and the Down arrow.
function Page-Paths([int]$Rows = 6) {
    $paths = @()
    for ($i = 0; $i -lt $Rows; $i++) {
        $mark = "zq-none-$($script:rnd.Next())"
        [void](Set-Clip $mark)
        [S]::Chord($VK.CTRL, $VK.SHIFT, $VK.C)
        $t = Wait-Until { $c = Get-Clip; if ($c -and $c -ne $mark) { $c } } 1.2 100
        if (-not $t) { break }
        $t = Clean ([string]$t)
        if ($paths -contains $t) { break }
        $paths += $t
        [S]::Chord($VK.DOWN)
        Start-Sleep -Milliseconds 90
    }
    if ($paths.Count -gt 1) { [S]::Chord($VK.CTRL, $VK.HOME); Start-Sleep -Milliseconds 60 }
    return , $paths
}
# Classic window: no type filter and no "inside documents" left on by an
# earlier step, so that a plain search is a plain search.
function Plain-Search-State {
    $ui = Find-Controls
    if (-not $ui) { return }
    if ($ui.Kind -and [S]::Msg($ui.Kind.Handle, 0x0147, 0, 0) -gt 0) {       # CB_GETCURSEL
        [S]::ComboPick($ui.Kind.Handle, 0)
        Log 'the type filter was not on "all types": put back'
        Start-Sleep -Milliseconds 250
    }
    if ($ui.Content -and [S]::Msg($ui.Content.Handle, $BM_GETCHECK, 0, 0) -eq 1) {
        [void][S]::Post($ui.Content.Handle, $BM_CLICK, 0, 0)
        Log '"inside documents" was ticked: unticked'
        Start-Sleep -Milliseconds 300
    }
}
# Types $Q in the main window and looks for the file $ExpectPath among the
# results. Returns whether it was found.
function Search-Expect([string]$Q, [string]$ExpectPath, [string]$Kind, [string]$Check, [int]$Wait = 15) {
    $name = Split-Path $ExpectPath -Leaf
    $base = [IO.Path]::GetFileNameWithoutExtension($name)
    if (-not (Focus-Query)) { Record 'search' $Kind $Check $false 'no main window or search box'; return $false }
    $onScreen = Look-Now
    if ($onScreen -eq 'classic') { Plain-Search-State; if (-not (Focus-Query)) { Record 'search' $Kind $Check $false 'no search box'; return $false } }
    Type-Query $Q
    $t = Get-Date
    $hit = $null; $seen = ''
    if ($onScreen -eq 'classic') {
        $hit = Wait-Until { Main-Names | Where-Object { $_ -like "*$base*" } | Select-Object -First 1 } $Wait 300
        $names = @(Main-Names 5)
        $seen = "rows: $(($names | Select-Object -First 3) -join ' | '); status: $(Status-Text)"
    } else {
        $paths = @()
        $end = (Get-Date).AddSeconds($Wait)
        do {
            Start-Sleep -Milliseconds 500
            $paths = Page-Paths 6
            $hit = $paths | Where-Object { (Split-Path $_ -Leaf) -eq $name } | Select-Object -First 1
        } while (-not $hit -and (Get-Date) -lt $end)
        $seen = "first rows (copied paths): $(($paths | Select-Object -First 3) -join ' | ')"
        # Core only when the rows could be read and the file is not among
        # them; rows that cannot be read prove nothing about the result.
        if (-not $hit -and -not ($script:pageReads -and $paths.Count -gt 0)) {
            $Kind = 'probe'
            $seen += ' (the page''s rows could not be read with the keyboard here: a finding about the test''s reach, not a wrong result)'
        }
    }
    $ok = $null -ne $hit
    Record 'search' $Kind $Check $ok ("'{0}' -> {1:N1} s; {2}" -f $Q, ((Get-Date) - $t).TotalSeconds, $seen)
    if (-not $ok) { Shot "search-miss-$Check" }
    return $ok
}
function Quick-Rows {
    if (-not (Test-Path $rowsFile)) { return @() }
    try { return [IO.File]::ReadAllLines($rowsFile, [Text.Encoding]::UTF8) } catch { return @() }
}
# The same through the Ctrl+Alt+F popup, whose rows the program writes to
# quick-rows.txt when BAHTH_UI_TEST is set.
function Quick-Expect([string]$Q, [string]$Expect, [string]$Kind, [string]$Check, [int]$Wait = 12) {
    $pop = Popup
    if (-not $pop) { [S]::Chord($VK.CTRL, $VK.ALT, $VK.F); $pop = Wait-Until { Popup } 5 80 }
    if (-not $pop) { Record 'quick search' 'probe' "$Check (popup opens)" $false 'Ctrl+Alt+F did not open the popup'; return $false }
    if ([S]::GetForegroundWindow() -ne $pop.Handle) { Focus-Window $pop.Handle }
    [S]::Chord($VK.CTRL, $VK.A); [S]::Chord($VK.BACK)
    [S]::Type($Q, 4, 10)
    $t = Get-Date
    $hit = Wait-Until {
        $rows = @(Quick-Rows)
        if ($rows.Count -gt 0 -and $rows[0] -eq "query`t$Q") { $rows | Select-Object -Skip 1 | Where-Object { (Clean $_) -like "*$Expect*" } | Select-Object -First 1 } else { $null }
    } $Wait 250
    $all = @(Quick-Rows)
    $shown = if ($all.Count -gt 0) { $all[0] -replace "`t", ': ' } else { 'no quick-rows.txt' }
    $ok = $null -ne $hit
    Record 'quick search' $Kind $Check $ok ("'{0}' -> {1:N1} s; {2}; rows: {3}" -f $Q, ((Get-Date) - $t).TotalSeconds, $shown, ((($all | Select-Object -Skip 1 | Select-Object -First 4) -replace "`t", ': ') -join ' | '))
    if (-not $ok) { Shot "quick-miss-$Check" }
    [void](Close-Popup)
    return $ok
}

# ---------------------------------------------------------------- random text
$U = { param([int]$Code) [char]::ConvertFromUtf32($Code) }
$emoji = @((& $U 0x1F600), (& $U 0x1F525), ((& $U 0x1F44D) + (& $U 0x1F3FD)), ((& $U 0x1F1EE) + (& $U 0x1F1F6)), ((& $U 0x1F468) + [char]0x200D + (& $U 0x1F469) + [char]0x200D + (& $U 0x1F467)), ([string][char]0x2764 + [char]0xFE0F))
$bidi = @([char]0x202E, [char]0x202D, [char]0x202B, [char]0x202C, [char]0x2066, [char]0x2067, [char]0x2068, [char]0x2069, [char]0x200F, [char]0x200E, [char]0x061C, [char]0x200D, [char]0x200C, [char]0xFEFF)
$words = @{
    ar      = @('تقرير', 'مالي', 'عقد', 'فاتورة', 'صورة', 'محاضرة', 'الفيزياء', 'أحمد', 'سيرة ذاتية', 'ميزانية ٢٠٢٦', 'مشروع', 'ملف', 'شركة', 'كتاب', 'إلى', 'ـــ', 'لَا إِلٰهَ', 'ﷺ', 'گچپژ')
    en      = @('report', 'budget', 'invoice', 'photo', 'final', 'draft v2', 'README', 'setup.exe', 'New Folder', 'Quarterly', 'windows', 'a', 'the', 'COM1', 'nul', 'x' )
    arabizi = @('ta2rer', 'mo7ammad', '3arabi', 'sa7', 'fatoora', 'el7amdulillah', 'shu2un', 'ma3lesh', '5alas', 'hgjrvdv', 'jrvdv lhgd')
    digits  = @('2026', '0', '000000', '123456789012345678901234567890', '٢٠٢٦', '۱۲۳', '3.14', '-1', '1e309', '0x7FFFFFFF', '2+2', '100 usd to iqd')
    symbols = @('!!!', '***', '???', '|||', '"""', '\\\\', '%%%', '()[]{}', '...', '~`^', '<>:"/\|?*', '&&', ';;', '--', '__', '@#$', "'", '!', '|', '" "', '""', '!|!', '| |', '*.*', '%TEMP%', '$env:x', '{0}', '\\?\C:\', 'C:\', '..\..\', '//')
    syntax  = @('ext:pdf', 'ext:', 'ext:PDF,,DOCX;', 'ext:jpg;png', 'size:>1mb', 'size:<1kb', 'size:>999999999999gb', 'size:', 'size:>-1', 'dm:today', 'dm:>1970-01-05', 'dm:', 'dm:9999-99-99', 'datemodified:yesterday', 'حجم:<1kb', 'تاريخ:today', 'مجلد:', 'folder:', 'folder: صور',
        '"exact phrase"', '"تقرير مالي"', '"unclosed', 'a|b', 'تقرير|report', '!old', 'عقد !قديم', 'صورة|فيديو', 'regex:(a+)+$', 'regex:[', 're:(', 're:.*.*.*.*x', 'regex:\', 'regex:السنو.', '! |', '| !', 'ext:pdf size:>1mb dm:today "تقرير" | !old', '.pdf', 'report .pdf')
}
function Pick($List) { $List[$script:rnd.Next($List.Count)] }
function Random-Query {
    $kind = $script:rnd.Next(100)
    if ($kind -lt 18) { return (Pick $words.ar) + ' ' + (Pick $words.ar) }
    if ($kind -lt 32) { return (Pick $words.en) + ' ' + (Pick $words.en) }
    if ($kind -lt 44) { return (Pick $words.ar) + ' ' + (Pick $words.en) + ' ' + (Pick $words.digits) }
    if ($kind -lt 52) { return (Pick $words.arabizi) }
    if ($kind -lt 60) { return (Pick $words.digits) }
    if ($kind -lt 68) { return (Pick $emoji) + (Pick $words.ar) + (Pick $emoji) }
    if ($kind -lt 78) { return (Pick $words.symbols) + (Pick $words.symbols) }
    if ($kind -lt 94) { return (Pick $words.syntax) + ' ' + (Pick $words.syntax) + ' ' + (Pick $words.ar) }
    if ($kind -lt 97) { return (Pick $words.ar) + [string](Pick $bidi) + (Pick $words.en) + [string](Pick $bidi) }
    # A long one.
    $s = ''
    for ($i = 0; $i -lt 30; $i++) { $s += (Pick $words.ar) + ' ' }
    return $s
}

# ---------------------------------------------------------------- scenario parts
function Same-Text([string]$A, [string]$B) { return ((Clean $A).Trim() -eq (Clean $B).Trim()) }

# Random queries typed fast, each cut short by the next.
function Typing-Storm([string]$Area, [int]$Count, [string]$Label) {
    if (-not (Focus-Query)) { Record $Area 'probe' "${Label}: the search box can be reached" $false 'no main window or search box'; return }
    $last = ''
    $t = Get-Date
    $done = 0
    for ($i = 1; $i -le $Count; $i++) {
        $q = Random-Query
        Select-Query
        [S]::Type($q, 12, $script:rnd.Next(0, 5))
        $last = $q
        $done++
        if ($i % 10 -eq 0) { Health "$Label, query $i" -Quick }
        if ($i % 50 -eq 0 -and $i -lt $Count) {
            Health "$Label, query $i"
            if ($script:abort) { return }
            # (A stray key may have hidden the window or moved the caret.)
            if (-not (Focus-Query)) { break }
        }
    }
    Start-Sleep -Milliseconds 500
    $shown = Read-Query
    $secs = ((Get-Date) - $t).TotalSeconds
    Note $Area "$Label" ("{0} queries in {1:N0} s ({2:N1} a second)" -f $done, $secs, ($done / [Math]::Max(1, $secs)))
    Record $Area 'probe' "${Label}: the box shows what was typed last" (($null -ne $shown) -and (Same-Text $shown $last)) "typed '$last'; the box holds '$shown'"
    Health $Label
}

function Backspace-Storm {
    if (-not (Focus-Query)) { return }
    for ($r = 0; $r -lt 5; $r++) {
        $text = ''
        for ($i = 0; $i -lt 10; $i++) { $text += (Random-Query) + ' ' }
        Select-Query
        [S]::Type($text, 24, 2)
        [S]::Tap($VK.BACK, $text.Length + 30, 0)
        Health "backspace storm $r" -Quick
    }
    Start-Sleep -Milliseconds 400
    if ((Look-Now) -eq 'classic') {
        $left = Read-Query
        Record 'typing' 'probe' 'the box is empty after the backspace storms' ([string]::IsNullOrEmpty($left)) "the box holds '$left'"
    }
    Health 'backspace storms'
}

function Hold-Keys {
    if (-not (Focus-Query)) { return }
    Clear-Query
    [S]::Hold($VK.A, 5000, 30)
    Health 'a letter held for 5 s'
    if ((Look-Now) -eq 'classic') { $ui = Find-Controls; if ($ui -and $ui.Query) { Note 'typing' 'a letter held for 5 s' "the box holds $([S]::TextLength($ui.Query.Handle)) characters" } }
    [S]::Hold($VK.BACK, 5000, 30)
    Health 'Backspace held for 5 s'
    # Down held: through the results (or the history menu of an empty box).
    Type-Query 'e'
    Start-Sleep -Milliseconds 500
    [S]::Hold($VK.DOWN, 3000, 30)
    Health 'Down held for 3 s'
    Close-Menus
    if ((Look-Now) -eq 'classic') {
        # Enter held in the search box: the same search again and again. (Not
        # in the new look: there Enter opens the selected result.)
        if (Focus-Query) {
            Type-Query 'zzqqxxnotthere'
            [S]::Hold($VK.ENTER, 3000, 30)
            Health 'Enter held for 3 s'
        }
    } else {
        [S]::Chord($VK.ESC)     # closes the history menu if Down opened it
        Start-Sleep -Milliseconds 150
    }
    [void](Dismiss-Dialogs 'no' 'holding keys')
    [void](Close-Opened 'holding keys')
    if (Focus-Query) { Clear-Query }
}

function Paste-Test([string]$Label, [string]$Text) {
    if (-not (Set-Clip $Text)) { Record 'typing' 'probe' "SCRIPT: clipboard set for the paste ($Label)" $false ''; return }
    if (-not (Focus-Query)) { return }
    Select-Query
    $t = Get-Date
    [S]::Chord($VK.CTRL, $VK.V)
    Start-Sleep -Milliseconds 900
    Health "paste: $Label"
    $held = ''
    if ((Look-Now) -eq 'classic') { $ui = Find-Controls; if ($ui -and $ui.Query) { $held = "; the box holds $([S]::TextLength($ui.Query.Handle)) characters" } }
    Note 'typing' "paste: $Label" ("{0:N0} characters pasted, the window answered again after {1:N1} s{2}" -f $Text.Length, ((Get-Date) - $t).TotalSeconds, $held)
    if (Focus-Query) { Clear-Query }
    Start-Sleep -Milliseconds 300
    Health "clearing after the paste: $Label"
}

function Clear-Storm {
    if (-not (Focus-Query)) { return }
    for ($i = 0; $i -lt 100; $i++) {
        [S]::Type((Pick $words.en), 12, 0)
        [S]::Chord($VK.CTRL, $VK.A)
        [S]::Chord($VK.DEL)
        Start-Sleep -Milliseconds 15
        if ($i % 25 -eq 24) { Health "clear $i" -Quick }
    }
    Health 'Ctrl+A, Delete 100 times'
}

# Hides the main window in the way named; returns whether it went.
function Hide-Main([string]$How) {
    $m = Shown-Main
    if (-not $m) { return $false }
    if ($How -eq 'altf4') {
        Focus-Window $m.Handle
        # Alt+F4 only when our window really has the keyboard.
        if ([S]::GetForegroundWindow() -eq $m.Handle) { [S]::Chord($VK.ALT, $VK.F4) } else { [void][S]::Post($m.Handle, $WM_SYSCOMMAND, $SC_CLOSE, 0) }
    } elseif ($How -eq 'esc') {
        if (Focus-Query) {
            Clear-Query
            for ($k = 0; $k -lt 3 -and (Look-Now) -ne 'none'; $k++) { [S]::Chord($VK.ESC); Start-Sleep -Milliseconds 200 }
        }
    } elseif ($How -eq 'menu' -and (Look-Now) -eq 'classic') {
        [void](Options-Choose $rx.Hide)
    } else {
        # The title bar's X.
        [void][S]::Post($m.Handle, $WM_SYSCOMMAND, $SC_CLOSE, 0)
    }
    return [bool](Wait-Until { (Look-Now) -eq 'none' } 3 60)
}

function Tray-Storm([int]$Times) {
    $hid = 0; $back = 0; $n = 0
    $ways = @('x', 'altf4', 'esc', 'x', 'menu')
    $tried = @{ x = 0; altf4 = 0; esc = 0; menu = 0 }
    $went = @{ x = 0; altf4 = 0; esc = 0; menu = 0 }
    # Where Windows shows the tray icon (it may sit in the overflow).
    $rect = [S]::TrayIconRect((Classic-Handle))
    Note 'tray' 'the tray icon' $(if ($rect) { "Windows reports it at $($rect[0]),$($rect[1]) ($($rect[2])x$($rect[3])); its menu is opened through the icon's own message" } else { 'Windows reports no rectangle for it (hidden or in the overflow); its menu is opened through the icon''s own message' })
    for ($i = 0; $i -lt $Times; $i++) {
        if (-not (Restore-Main)) { break }
        $n++
        $way = $ways[$i % 5]
        $tried[$way]++
        if (Hide-Main $way) { $hid++; $went[$way]++ } else { Log "tray storm ${i}: '$way' did not hide the window" }
        Health "hide $i" -Quick
        if ($i -eq 0) { Shot 'hidden-to-tray' }
        if ($i % 5 -eq 2) { [void](Second-Launch) } else { [void](Tray-Choose $rx.TrayFull) }
        if (Wait-Until { Shown-Main } 4 60) { $back++ }
        else {
            Log "tray storm ${i}: the window did not come back; starting the exe again"
            [void](Second-Launch)
            [void](Wait-Until { Shown-Main } 6 100)
        }
        if ($i % 10 -eq 9) { Health "tray storm $i" }
        if ($script:abort) { return }
    }
    $perWay = (@('x', 'altf4', 'esc', 'menu') | ForEach-Object { "$_ $($went[$_])/$($tried[$_])" }) -join ', '
    Record 'tray' 'probe' "hides to the tray $Times times (X, Alt+F4, Esc, Options > Hide)" ($hid -eq $n -and $n -eq $Times) "$hid of $n ($perWay; in the new look 'menu' is the X again)"
    Record 'tray' 'probe' "comes back $Times times (tray menu, second start)" ($back -eq $n -and $n -eq $Times) "$back of $n"
    Record 'tray' 'probe' 'the floating box stays through it' ($null -ne (Visible-Window 'InstaFindFloatingBox')) ''
    $one = Wait-Until { @(Bahth-Procs).Count -eq 1 } 10 250
    Record 'tray' 'probe' 'one process after the second starts' ([bool]$one) "$(@(Bahth-Procs).Count) bahth processes"
}

function Window-Storm {
    $m = Restore-Main
    if (-not $m) { return }
    $h = $m.Handle
    $scr = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    for ($i = 0; $i -lt 30; $i++) {
        foreach ($cmd in @($SC_MINIMIZE, $SC_RESTORE, $SC_MAXIMIZE, $SC_RESTORE)) { [void][S]::Post($h, $WM_SYSCOMMAND, $cmd, 0); Start-Sleep -Milliseconds 35 }
        if ($i % 10 -eq 9) { Health "minimise/maximise $i" -Quick }
    }
    Health 'minimise, maximise, restore 30 times'
    $sizes = @(@(1, 1), @(120, 80), @(300, 200), @(4000, 3000), @(640, 480), @(50, 600), @(900, 30), @($scr.Width, $scr.Height), @(0, 0), @(801, 601))
    for ($i = 0; $i -lt 40; $i++) {
        $s = $sizes[$i % $sizes.Count]
        [S]::Place($h, 8 + ($i % 7) * 6, 8 + ($i % 5) * 4, $s[0], $s[1])
        Start-Sleep -Milliseconds 35
        if ($i % 10 -eq 9) { Health "resize $i" -Quick }
    }
    Health 'resized 40 times (also to tiny sizes)'
    $spots = @(@(-300, -200), @(($scr.Width - 80), ($scr.Height - 60)), @(0, 0), @(5000, 5000), @(-5000, 40), @(40, 40))
    for ($i = 0; $i -lt 30; $i++) {
        $p = $spots[$i % $spots.Count]
        [S]::Place($h, $p[0], $p[1], 820, 560)
        Start-Sleep -Milliseconds 35
    }
    Health 'moved 30 times (also off the screen)'
    # Dragged by its title bar, as a person does.
    [S]::Place($h, 60, 60, 820, 560)
    Start-Sleep -Milliseconds 200
    Focus-Window $h
    for ($i = 0; $i -lt 5; $i++) {
        $w = Win-Of $h
        if (-not $w) { break }
        $dx = if ($i % 2) { -90 } else { 110 }
        [S]::Drag([int]($w.X + $w.W / 2), ($w.Y + 14), [int]($w.X + $w.W / 2 + $dx), ($w.Y + 14 + 50))
        Start-Sleep -Milliseconds 150
    }
    Health 'dragged by the title bar 5 times'
    [void][S]::Post($h, $WM_SYSCOMMAND, $SC_RESTORE, 0)
    Start-Sleep -Milliseconds 200
    Place-Main
    Start-Sleep -Milliseconds 400
    $w = Win-Of $h
    Record 'window' 'probe' 'the window is back at a normal size after the abuse' ($w -and $w.Visible -and -not [S]::IsIconic($h) -and $w.W -gt 600 -and $w.H -gt 400) $(if ($w) { "$($w.W)x$($w.H) at $($w.X),$($w.Y)" } else { 'no window' })
    if ((Look-Now) -eq 'classic') {
        $ui = Find-Controls
        Record 'window' 'probe' 'search box, list and status bar are still there' ($ui -and $ui.Query -and $ui.List -and $ui.Status) ''
    } elseif ($w) {
        $p = Painted $w
        Record 'window' 'probe' 'the page is still painted' ($p -and $p.Colours -ge 8 -and $p.Top -lt 0.97) $(if ($p) { "$($p.Colours) colours, most common $($p.Top)" } else { 'no screenshot' })
    }
    Shot 'after-window-storm'
}

function Popup-Storm([int]$Times) {
    $opened = 0; $closed = 0; $n = 0
    for ($i = 0; $i -lt $Times; $i++) {
        $n++
        [S]::Chord($VK.CTRL, $VK.ALT, $VK.F)
        $p = Wait-Until { Popup } 2 40
        if ($p) {
            $opened++
            if ($i % 5 -eq 0) { [S]::Type((Random-Query), 6, 3) }
            if ($i -eq 0) { Shot 'quick-popup' }
            # Esc, or the hotkey again; both close it while it has the keyboard.
            if ([S]::GetForegroundWindow() -eq $p.Handle) {
                if ($i % 2) { [S]::Chord($VK.CTRL, $VK.ALT, $VK.F) } else { [S]::Chord($VK.ESC) }
            }
            if (Wait-Until { -not (Popup) } 2 40) { $closed++ } else { [void](Close-Popup) }
        }
        if ($i % 10 -eq 9) { Health "popup $i" -Quick }
        if ($opened -eq 0 -and $i -ge 4) { break }
    }
    Record 'quick popup' 'probe' "Ctrl+Alt+F opens the popup $Times times" ($opened -eq $Times) "$opened of $n"
    Record 'quick popup' 'probe' 'and it closes each time (Esc or the hotkey)' ($closed -eq $opened -and $opened -gt 0) "$closed of $opened"
    [void](Close-Popup)
}

function Box-Storm([int]$Times) {
    $box = Visible-Window 'InstaFindFloatingBox'
    if (-not $box) { Record 'floating box' 'probe' 'the floating box is on screen' $false 'no InstaFindFloatingBox window (floating_mode=fade was pre-set)'; return }
    for ($i = 0; $i -lt $Times; $i++) {
        $b = Visible-Window 'InstaFindFloatingBox'
        if ($b) { [S]::Click([int]($b.X + $b.W / 2), [int]($b.Y + $b.H * 0.65), $false) }
        Start-Sleep -Milliseconds 110
        if ($i % 10 -eq 9) { Health "box click $i" -Quick }
    }
    Start-Sleep -Seconds 1
    Record 'floating box' 'probe' "the box is still shown after $Times quick clicks" ($null -ne (Visible-Window 'InstaFindFloatingBox')) ''
    Shot 'after-box-clicks'
    [void](Close-Popup)
}

function Options-Storm([int]$Times) {
    $ok = 0
    for ($i = 0; $i -lt $Times; $i++) {
        $m = Open-Options
        if ($m) {
            $ok++
            if ($i -eq 0) { Note 'menus' 'Options items' ((Menu-Items $m | Where-Object { -not $_.Separator } | ForEach-Object Text) -join ' / '); Shot 'options-menu' }
            if ($i % 2) { Close-Menus -Escape } else { Close-Menus }
        }
        if ($i % 10 -eq 9) { Health "Options menu $i" -Quick }
    }
    Record 'menus' 'probe' "the Options menu opens $Times times" ($ok -eq $Times) "$ok of $Times"
    if ((Look-Now) -eq 'none') { [void](Restore-Main) }
}

# The switches the rest of the test relies on, read from settings.txt (the
# program saves them at once): Ctrl+Alt+F opens the popup, the floating box
# fades in, the tray icon is shown, Alt+Space is off. One that was left the
# other way round is chosen once more.
function Settle-Anywhere([string]$Area) {
    $want = @(@('hotkey_opens', 'quick', 4), @('floating_mode', 'fade', 0), @('tray_icon', '1', 6), @('alt_space', '0', 5))
    $wrong = @()
    foreach ($w in $want) {
        $now = Setting $w[0]
        if ($null -ne $now -and $now -ne $w[1]) {
            $wrong += "$($w[0])=$now"
            [void](Options-Choose '' $rx.Anywhere $w[2])
            Start-Sleep -Milliseconds 300
            [void](Dismiss-Dialogs 'no' "putting $($w[0]) back")
        }
    }
    $still = @($want | Where-Object { $now = Setting $_[0]; $null -ne $now -and $now -ne $_[1] } | ForEach-Object { "$($_[0])=$(Setting $_[0])" })
    Record $Area 'probe' 'hotkey, floating box, tray icon and Alt+Space are as before (settings.txt)' ($wrong.Count -eq 0) $(if ($wrong.Count) { "left changed: $($wrong -join ', '); after choosing them again: $(if ($still.Count) { $still -join ', ' } else { 'all back' })" } else { '' })
}

function Toggle-Storm {
    $before = Options-State
    if (-not $before) { Record 'options' 'probe' 'the Options menu can be read' $false ''; return }
    $bad = @(); $n = 0
    foreach ($pat in $rxToggles) {
        $a = Options-Choose $pat
        if (-not $a) { Note 'options' 'switch not in the menu' $pat; continue }
        Start-Sleep -Milliseconds 200
        [void](Dismiss-Dialogs 'no' "switch $($a.Text)")
        Health "switch $($a.Text)" -Quick
        $b = Options-Choose $pat
        [void](Dismiss-Dialogs 'no' "switch $($a.Text) back")
        $n++
        if (-not $b -or $b.Checked -eq $a.Checked) { $bad += $a.Text }
        Health "switch: $($a.Text)"
        if ($script:abort) { return }
    }
    # Options > Search from anywhere: the box hidden, off, back to fading in;
    # its place reset; the hotkey, Alt+Space and the tray icon off and on.
    $subOk = 0
    $subs = @(1, 2, 0, 3, 4, 4, 5, 5, 6, 6)
    foreach ($pos in $subs) {
        $x = Options-Choose '' $rx.Anywhere $pos
        if ($x) { $subOk++ }
        Start-Sleep -Milliseconds 200
        [void](Dismiss-Dialogs 'no' "search from anywhere, item $pos")
        Health "search from anywhere, item $pos" -Quick
    }
    Health 'Options switches'
    $after = Options-State
    $skipped = @($before | Where-Object { $t = $_; -not ($rxToggles | Where-Object { $t -match $_ }) } | ForEach-Object { $_.Substring(4) })
    Note 'options' 'items left alone' ($skipped -join ' / ')
    Record 'options' 'probe' "each of the $n switches flips when chosen" ($bad.Count -eq 0 -and $n -gt 0) $(if ($bad.Count) { 'did not flip: ' + ($bad -join ' / ') } else { '' })
    Record 'options' 'probe' 'the 7 items under "Search from anywhere" can be chosen' ($subOk -eq $subs.Count) "$subOk of $($subs.Count) choices"
    $same = $after -and (($before -join '|') -eq ($after -join '|'))
    $diff = if ($after) { (Compare-Object $before $after | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }) -join ' ; ' } else { 'menu did not open' }
    Record 'options' 'probe' 'every switch is back as it was' ([bool]$same) $diff
    Settle-Anywhere 'options'
    Record 'options' 'probe' 'the floating box is back after "off"' ($null -ne (Wait-Until { Visible-Window 'InstaFindFloatingBox' } 5)) ''
}

# New look: the switches of the page's settings (Ctrl+,), reached with Tab.
# Blind: the page cannot be read here, so the first switch (Arabic-Indic
# digits, 3 Tabs from the "back" button) is checked through the classic
# Options menu, which the program keeps in step; only if that works are the
# others pressed. 1.0's page, in Tabs from "back" (src/webui/assets/app.js,
# renderSettings): 1-2 language, 3 digits, 4 compact rows, 5 hide system
# files, 6 document contents, 7 cloud-only files, 8 update automatically,
# 9 "update index", 10 hotkey, 11 box above the clock, 12 tray icon,
# 13 start with Windows, 14 "check for updates", 15 "report a problem",
# 16 "more options". Never pressed: the language (a restart), 7 (it would
# download cloud files where there are any), 9, 14 (the network), 15, 16.
function Page-Switch([int]$Tabs) {
    if (-not (Focus-Query)) { return }
    [S]::Chord($VK.CTRL, $VK.COMMA)
    Start-Sleep -Milliseconds 300
    [S]::Tap($VK.TAB, $Tabs, 50)
    [S]::Chord($VK.SPACE)
    Start-Sleep -Milliseconds 350
    [S]::Chord($VK.CTRL, $VK.K)     # back to the search box (closes the settings)
    Start-Sleep -Milliseconds 150
}
function Page-Settings-Storm {
    $digits = { $s = Options-State; if ($s) { $s | Where-Object { $_ -match $rx.Digits } | Select-Object -First 1 } }
    $before = Options-State
    $d0 = & $digits
    Page-Switch 3
    Shot 'page-settings'
    $d1 = & $digits
    $works = $d0 -and $d1 -and ($d0 -ne $d1)
    Record 'page settings' 'probe' 'a switch on the settings page can be flipped with the keyboard (Ctrl+, then Tab, Space)' ([bool]$works) "digits before: $d0; after: $d1"
    if (-not $works) { return }
    Page-Switch 3
    foreach ($tabs in @(4, 5, 6, 8, 10, 11, 12, 13)) {
        Page-Switch $tabs
        Health "page switch $tabs" -Quick
        Page-Switch $tabs
        Health "page switch $tabs back"
    }
    $after = Options-State
    $same = $after -and (($before -join '|') -eq ($after -join '|'))
    $diff = if ($after -and $before) { (Compare-Object $before $after | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }) -join ' ; ' } else { 'menu did not open' }
    Record 'page settings' 'probe' 'every switch flipped twice is back as it was (seen in the Options menu)' ([bool]$same) $diff
    Settle-Anywhere 'page settings'
}

function Dialog-Storm {
    # With results in the list, "duplicates" starts from them instead of
    # asking for a folder.
    if (Focus-Query) { Type-Query 'zqk'; Start-Sleep -Milliseconds 600 }
    $opened = 0; $closed = 0; $n = 0; $first = @()
    foreach ($pat in $rxDialogs) {
        for ($k = 0; $k -lt 2; $k++) {
            $n++
            $it = Options-Choose $pat
            if (-not $it) { Note 'dialogs' 'item not in the menu' $pat; break }
            $up = Wait-Until { if (@(Extra-Windows).Count -or @(Dialogs).Count) { 'up' } } 4 100
            $names = (@(Extra-Windows | ForEach-Object { $_.Text }) + @(Dialogs | ForEach-Object { Dialog-Text $_ })) -join ' / '
            if ($up) { $opened++; if ($k -eq 0) { $first += $names; Shot "dialog-$($n)" } } else { Log "dialog '$pat' did not open" }
            Health "dialog: $pat" -Quick
            [void](Dismiss-Dialogs 'no' "dialog $pat")
            [void](Close-Extras)
            [void](Dismiss-Dialogs 'no' "dialog $pat, after closing")
            if (Wait-Until { -not (@(Extra-Windows).Count -or @(Dialogs).Count) } 4 100) { $closed++ }
            Health "dialog closed: $pat"
            if ($script:abort) { return }
        }
    }
    Note 'dialogs' 'windows seen' ($first -join ' ; ')
    Record 'dialogs' 'probe' 'duplicates, folder sizes, replace text and phone search each open twice' ($opened -eq 8) "$opened of $n opened"
    Record 'dialogs' 'probe' 'and each closes again' ($closed -eq $n) "$closed of $n closed"
    if ((Look-Now) -eq 'none') { [void](Restore-Main) }
}

# Classic window: sorting by every column, the type filter, search inside
# documents switched mid-search, stop.
function Classic-Filters {
    if (-not (Focus-Query)) { return }
    Type-Query 'zqk'
    [void](Wait-Until { Main-Names 20 | Where-Object { $_ -like 'zqk*' } | Select-Object -First 1 } 10 300)
    $ui = Find-Controls
    if (-not $ui -or -not $ui.List) { Record 'results' 'probe' 'the result list is there' $false ''; return }
    $clicks = 0
    for ($round = 0; $round -lt 2; $round++) {
        foreach ($col in 0..3) {
            for ($k = 0; $k -lt 3; $k++) {
                $r = [S]::HeaderRect($ui.List.Handle, $col)
                if ($r -and $r[2] -gt 8) { [S]::Click([int]($r[0] + $r[2] / 2), [int]($r[1] + $r[3] / 2), $false); $clicks++ }
                Start-Sleep -Milliseconds 110
            }
            Health "sort by column $col" -Quick
        }
    }
    Health 'sorting by every column, 24 clicks'
    Record 'results' 'probe' 'the four column headers can be clicked' ($clicks -eq 24) "$clicks of 24 clicks"
    $names = @(Main-Names)
    $missing = @($script:keep | Where-Object { $b = [IO.Path]::GetFileNameWithoutExtension($_.Path); -not ($names | Where-Object { $_ -like "*$b*" }) } | ForEach-Object { $_.What })
    Record 'results' $script:resultKind 'sorting keeps every result (the 5 test files are still listed)' ($missing.Count -eq 0) $(if ($missing.Count) { "missing: $($missing -join ', '); rows: $($names.Count)" } else { "$($names.Count) rows" })
    Shot 'after-sorting'

    # The type filter, picked quickly.
    if ($ui.Kind) {
        $count = [int][S]::Msg($ui.Kind.Handle, 0x0146, 0, 0)      # CB_GETCOUNT
        for ($i = 0; $i -lt 20; $i++) { [S]::ComboPick($ui.Kind.Handle, $script:rnd.Next([Math]::Max(1, $count))); Start-Sleep -Milliseconds 50 }
        # And with the mouse and the keyboard.
        for ($i = 0; $i -lt 4; $i++) {
            Click-Center $ui.Kind; Start-Sleep -Milliseconds 150
            [S]::Chord($VK.DOWN); Start-Sleep -Milliseconds 60
            [S]::Chord($VK.ENTER); Start-Sleep -Milliseconds 120
        }
        [S]::ComboPick($ui.Kind.Handle, 0)
        Start-Sleep -Milliseconds 300
        $cur = [int][S]::Msg($ui.Kind.Handle, 0x0147, 0, 0)        # CB_GETCURSEL
        Record 'results' 'probe' 'the type filter is back on "all types"' ($cur -eq 0) "selected entry $cur of $count"
        Health 'type filter changed 24 times'
    }

    # Search inside documents switched on and off while a search runs.
    if ($ui.Content) {
        if (Focus-Query) { Type-Query 'report' }
        [void][S]::Post($ui.Content.Handle, $BM_CLICK, 0, 0)
        Start-Sleep -Milliseconds 200
        [S]::Chord($VK.ENTER)
        Start-Sleep -Milliseconds 600
        for ($i = 0; $i -lt 10; $i++) { [void][S]::Post($ui.Content.Handle, $BM_CLICK, 0, 0); Start-Sleep -Milliseconds 140 }
        Health 'inside documents switched 10 times mid-search'
        # Stop mid-way, three times.
        for ($i = 0; $i -lt 3; $i++) {
            if ([S]::Msg($ui.Content.Handle, $BM_GETCHECK, 0, 0) -ne 1) { [void][S]::Post($ui.Content.Handle, $BM_CLICK, 0, 0); Start-Sleep -Milliseconds 150 }
            if (Focus-Query) { Type-Query (@('report', 'the', 'تقرير')[$i]); [S]::Chord($VK.ENTER) }
            Start-Sleep -Milliseconds 900
            $ui2 = Find-Controls
            if ($ui2 -and $ui2.Stop) { [void][S]::Post($ui2.Stop.Handle, $BM_CLICK, 0, 0) }
            Start-Sleep -Milliseconds 400
            if ($i -eq 0) { Note 'results' 'after "Stop search"' (Status-Text); Shot 'search-stopped' }
            Health "search stopped mid-way $i"
        }
        for ($i = 0; $i -lt 3 -and [S]::Msg($ui.Content.Handle, $BM_GETCHECK, 0, 0) -ne 0; $i++) { [void][S]::Post($ui.Content.Handle, $BM_CLICK, 0, 0); Start-Sleep -Milliseconds 250 }
        Record 'results' 'probe' '"inside documents" is unticked again' ([S]::Msg($ui.Content.Handle, $BM_GETCHECK, 0, 0) -eq 0) ''
    }
    [void](Dismiss-Dialogs 'no' 'filters')
}

# New look: the same with the keyboard, blind (Tab order of the page: search
# box, history, settings, then the chips: type 3, size 4, date 5, folder 6,
# inside documents 7, sort 8). The query only matches the test's own files,
# so a key that lands elsewhere can open nothing but one of them.
function Page-Filters {
    if (-not (Focus-Query)) { return }
    Type-Query 'zqk'
    Start-Sleep -Milliseconds 700
    for ($i = 0; $i -lt 8; $i++) {
        # The type chip's menu: Enter opens it, Down, Enter picks (8 types: back to "all").
        [S]::Chord($VK.CTRL, $VK.K); [S]::Tap($VK.TAB, 3, 40); [S]::Chord($VK.ENTER); Start-Sleep -Milliseconds 120
        [S]::Chord($VK.DOWN); Start-Sleep -Milliseconds 50; [S]::Chord($VK.ENTER); Start-Sleep -Milliseconds 150
        Health "page type filter $i" -Quick
    }
    Health 'page: type filter changed 8 times'
    for ($i = 0; $i -lt 8; $i++) {
        [S]::Chord($VK.CTRL, $VK.K); [S]::Tap($VK.TAB, 8, 40); [S]::Chord($VK.ENTER); Start-Sleep -Milliseconds 120
        [S]::Chord($VK.DOWN); Start-Sleep -Milliseconds 50; [S]::Chord($VK.ENTER); Start-Sleep -Milliseconds 150
        Health "page sort $i" -Quick
    }
    Health 'page: sort order changed 8 times'
    # Inside documents on and off while it searches (the chip starts the
    # search by itself when the box is not empty), then Esc stops it.
    for ($i = 0; $i -lt 10; $i++) { [S]::Chord($VK.CTRL, $VK.K); [S]::Tap($VK.TAB, 7, 40); [S]::Chord($VK.SPACE); Start-Sleep -Milliseconds 160 }
    Health 'page: inside documents switched 10 times'
    for ($i = 0; $i -lt 3; $i++) {
        [S]::Chord($VK.CTRL, $VK.K); [S]::Tap($VK.TAB, 7, 40); [S]::Chord($VK.SPACE); Start-Sleep -Milliseconds 900
        [S]::Chord($VK.ESC); Start-Sleep -Milliseconds 300
        [S]::Chord($VK.CTRL, $VK.K); [S]::Tap($VK.TAB, 7, 40); [S]::Chord($VK.SPACE); Start-Sleep -Milliseconds 200
        Health "page: search inside documents stopped $i"
        if ((Look-Now) -ne 'new') { [void](Restore-Main) }
        if (-not (Focus-Query)) { return }
        Type-Query 'zqk'
        Start-Sleep -Milliseconds 300
    }
    Shot 'page-after-filters'
    [void](Dismiss-Dialogs 'no' 'page filters')
    [void](Close-Opened 'page filters')
}

# Stops the program, writes clean settings for the look under test and starts
# it again (the index is kept).
function Reset-App([string]$Why) {
    Log "reset: $Why"
    Keep-Log "before reset: $Why"
    Stop-App $Why
    Write-Settings $(if ($Look -eq 'new') { 1 } else { 0 })
    $w = Start-App 60
    if (-not $w) { Record 'stability' 'core' "the program starts again ($Why)" $false 'no main window within 60 s'; $script:abort = $true; return $false }
    if ($Look -eq 'new') { Start-Sleep -Seconds 5 } else { Start-Sleep -Seconds 2 }
    return $true
}

# ---- actions on results that changed behind the program's back
# Selects the first result and returns its full path ('' when it could not).
function First-Result([string]$Q) {
    if (-not (Focus-Query)) { return '' }
    Type-Query $Q
    if ((Look-Now) -eq 'classic') {
        if (-not (Wait-Until { Main-Names 5 | Where-Object { $_ -like "*$Q*" } | Select-Object -First 1 } 10 300)) { return '' }
        $ui = Find-Controls
        if (-not $ui -or -not $ui.List) { return '' }
        [S]::Click([int]($ui.List.X + $ui.List.W / 2), ($ui.List.Y + 36), $false)
        Start-Sleep -Milliseconds 200
        $sel = [S]::ListSelected($ui.List.Handle)
        if ($sel -lt 0) { return '' }
        return (Clean ([S]::ListText($ui.List.Handle, $sel, 1)))
    }
    Start-Sleep -Milliseconds 700
    $paths = Page-Paths 1
    if ($paths.Count) { return [string]$paths[0] }
    return ''
}
function Act-On-Result([string]$Action, [string]$NewName = 'zqrenamed') {
    $classic = (Look-Now) -eq 'classic'
    if ($Action -eq 'enter') { [S]::Chord($VK.ENTER) }
    elseif ($Action -eq 'ctrl-enter') { [S]::Chord($VK.CTRL, $VK.ENTER) }
    elseif ($Action -eq 'f2') { [S]::Chord($VK.F2); Start-Sleep -Milliseconds 350; [S]::Type($NewName, 6, 5); Start-Sleep -Milliseconds 100; [S]::Chord($VK.ENTER) }
    elseif ($Action -eq 'f4') { [S]::Chord($VK.F4) }
    elseif ($Action -eq 'ctrl-c') { [S]::Chord($VK.CTRL, $VK.C) }
    elseif ($Action -eq 'delete') {
        if ($classic) { [S]::Chord($VK.DEL) }
        else {
            # The page's Delete works in the list only; its menu is reachable
            # from the box: Shift+F10, End (the last item), Enter, then Enter
            # on the program's own question.
            [S]::Chord($VK.SHIFT, $VK.F10); Start-Sleep -Milliseconds 250
            [S]::Chord($VK.END); Start-Sleep -Milliseconds 80
            [S]::Chord($VK.ENTER); Start-Sleep -Milliseconds 300
            [S]::Chord($VK.ENTER)
        }
    }
}
function After-Action([string]$Why, [string]$Answer = 'no') {
    Start-Sleep -Milliseconds 700
    $said = Dismiss-Dialogs $Answer $Why
    $extra = Close-Extras
    $n = Close-Opened $Why
    if ((Look-Now) -eq 'new') { [S]::Chord($VK.ESC); Start-Sleep -Milliseconds 120 }   # a page dialog, if one is open (else it only clears the box)
    $status = if ((Look-Now) -eq 'classic') { Status-Text } else { '' }
    return (Flat ("status: $status; message boxes: $($said -join ' // '); windows closed: $($extra -join ', ') $n") 300)
}
function Gone-Rounds {
    $rounds = @()
    foreach ($change in @('deleted', 'renamed')) { foreach ($a in @('enter', 'ctrl-enter', 'f2', 'f4', 'delete', 'ctrl-c')) { $rounds += , @($a, $change) } }
    $done = 0; $notes = @()
    foreach ($r in $rounds) {
        $action = $r[0]; $change = $r[1]
        $path = First-Result 'zqvictim'
        if (-not $path -or -not $path.StartsWith($scratch, [StringComparison]::OrdinalIgnoreCase)) { Log "gone round $action/${change}: no usable first result ('$path')"; continue }
        try {
            if ($change -eq 'deleted') { Remove-Item -LiteralPath $path -Force -ErrorAction Stop }
            else { Rename-Item -LiteralPath $path -NewName ('zqmoved-' + [IO.Path]::GetFileName($path) + '.bak') -ErrorAction Stop }
        } catch { Log "gone round: could not change $path ($_)"; continue }
        Act-On-Result $action
        $said = After-Action "$action on a $change file"
        $done++
        $notes += "$action/${change}: $said"
        Log "gone round $action on a $change file ($path): $said"
        Health "$action on a file $change after the search"
        if ($script:abort) { return }
    }
    Record 'results' 'probe' 'Enter, Ctrl+Enter, F2, F4, Delete and Ctrl+C were tried on a deleted and on a renamed result' ($done -eq 12) "$done of 12 rounds ran"
    Note 'results' 'what the program said' (($notes | Select-Object -First 6) -join ' ; ')
    Shot 'after-gone-rounds'
}
# The same keys on files that are still there: rename, Recycle Bin, copy.
function Live-Actions {
    $path = First-Result 'zqren'
    if ($path -and $path.StartsWith($scratch, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $path)) {
        $classic = (Look-Now) -eq 'classic'
        # (The classic box holds the whole name; the page's selects the name
        # without its extension.)
        $typed = if ($classic) { 'zqdone-file.txt' } else { 'zqdone-file' }
        Act-On-Result 'f2' $typed
        $new = Join-Path $scratch 'zqdone-file.txt'
        $ok = Wait-Until { (Test-Path -LiteralPath $new) -and -not (Test-Path -LiteralPath $path) } 5 250
        $said = After-Action 'rename'
        Record 'results' 'probe' 'F2 renames a result (the file has the new name on disk)' ([bool]$ok) "$path -> $new; $said"
        Health 'F2 rename'
    } else { Record 'results' 'probe' 'F2 renames a result' $false "no usable first result for 'zqren' ('$path')" }

    $path = First-Result 'zqdel'
    if ($path -and $path.StartsWith($scratch, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $path)) {
        Act-On-Result 'delete'
        Start-Sleep -Milliseconds 600
        # Windows' own question, when its Recycle Bin is set to ask.
        [void](Dismiss-Dialogs 'yes' 'Recycle Bin question')
        $ok = Wait-Until { -not (Test-Path -LiteralPath $path) } 6 250
        $said = After-Action 'delete'
        Record 'results' 'probe' 'Delete moves a result to the Recycle Bin (the file is gone from its folder)' ([bool]$ok) "$path; $said"
        Health 'Delete'
    } else { Record 'results' 'probe' 'Delete moves a result to the Recycle Bin' $false "no usable first result for 'zqdel' ('$path')" }

    $path = First-Result 'zqk'
    if ($path) {
        [void](Set-Clip 'zq-before-copy')
        Act-On-Result 'ctrl-c'
        Start-Sleep -Milliseconds 500
        $files = @()
        try { $files = @(Get-Clipboard -Format FileDropList -ErrorAction Stop | ForEach-Object { $_.FullName }) } catch { Log "clipboard file list: $_" }
        Record 'results' 'probe' 'Ctrl+C copies the selected file (a file on the clipboard)' ($files.Count -gt 0) "result: $path; clipboard files: $($files -join ', ')"
        [void](After-Action 'copy')
        Health 'Ctrl+C'
    }
}
# 30 results opened one after the other, quickly. The editor that opens takes
# the keyboard each time, so the window is put in front again before each.
function Open-Many([int]$Count) {
    $path = First-Result 'zqopen'
    if (-not $path) { Record 'results' 'probe' "opens $Count files quickly" $false "no first result for 'zqopen'"; return }
    $m = Shown-Main
    $padsBefore = @(Get-Process -Name notepad -ErrorAction SilentlyContinue).Count
    $winsBefore = @([S]::VisibleHandles()).Count
    for ($i = 0; $i -lt $Count; $i++) {
        Focus-Window $m.Handle
        if ((Look-Now) -ne 'classic') {
            $c = [S]::ClientOnScreen($m.Handle)
            [S]::Click([int]($c[0] + $c[2] / 2), $c[1] + 34, $false)
            Start-Sleep -Milliseconds 80
        }
        [S]::Chord($VK.ENTER)
        Start-Sleep -Milliseconds 150
        Focus-Window $m.Handle
        [S]::Chord($VK.DOWN)
        Start-Sleep -Milliseconds 60
        if ($i % 10 -eq 9) { Health "opening file $i" -Quick }
    }
    Start-Sleep -Seconds 2
    Health "$Count files opened quickly"
    $pads = @(Get-Process -Name notepad -ErrorAction SilentlyContinue).Count - $padsBefore
    $wins = @([S]::VisibleHandles()).Count - $winsBefore
    Shot 'after-opening-files'
    $said = Dismiss-Dialogs 'no' 'opening many files'
    Record 'results' 'probe' "Enter on $Count results opens them (at least one editor window appeared)" ($pads -gt 0 -or $wins -gt 0) "$pads new Notepad process(es), $wins new window(s); message boxes: $($said -join ' // ')"
    $closed = Close-Opened 'opening many files'
    Start-Sleep -Milliseconds 800
    [void](Close-Opened 'opening many files, again')
    Note 'results' 'cleaning up after opening files' "$closed window(s)/process(es) the test opened were closed"
}
function Context-Storm([int]$Times) {
    $path = First-Result 'zqk'
    if (-not $path) { Record 'menus' 'probe' "the right-click menu opens $Times times" $false "no first result for 'zqk'"; return }
    if ((Look-Now) -eq 'classic') {
        $ui = Find-Controls
        $x = [int]($ui.List.X + $ui.List.W / 2); $y = $ui.List.Y + 36
        $shown = 0
        for ($i = 0; $i -lt $Times; $i++) {
            Focus-Window $ui.Main.Handle
            [S]::Click($x, $y, $false); Start-Sleep -Milliseconds 60
            [S]::Click($x, $y, $true)
            $menu = Wait-Menu 2
            if ($menu) {
                $shown++
                if ($i -eq 0) { Note 'menus' 'right-click items' ((Menu-Items $menu | Where-Object { -not $_.Separator } | ForEach-Object Text) -join ' / '); Shot 'right-click-menu' }
                if ($i % 2) { Close-Menus -Escape } else { Close-Menus }
            }
            if ($i % 10 -eq 9) { Health "right-click $i" -Quick }
        }
        Record 'menus' 'probe' "the right-click menu opens $Times times" ($shown -eq $Times) "$shown of $Times"
        return
    }
    # New look: Shift+F10 opens the page's menu for the selected row, Esc
    # closes it. The box is typed again each time, so an Esc that finds no
    # menu only clears the box and never hides the window.
    for ($i = 0; $i -lt $Times; $i++) {
        Type-Query 'zqk'
        Start-Sleep -Milliseconds 160
        [S]::Chord($VK.SHIFT, $VK.F10)
        Start-Sleep -Milliseconds 90
        if ($i -eq 0) { Start-Sleep -Milliseconds 300; Shot 'page-right-click-menu' }
        [S]::Chord($VK.ESC)
        Start-Sleep -Milliseconds 60
        if ($i % 10 -eq 9) { Health "page menu $i" -Quick; if ((Look-Now) -ne 'new') { [void](Restore-Main); [void](Focus-Query) } }
    }
    Note 'menus' "the page's row menu (Shift+F10, Esc) $Times times" 'blind: see the screenshot page-right-click-menu; only crashes and hangs are checked'
}

# ---- the new look only (1.0)
# There is no way back to the classic look: neither the Options menu over
# the new look (the tray's "Settings…" opens it) nor the tray menu offers it.
function No-Way-Back {
    if ((Look-Now) -ne 'new') { if (-not (Reset-App 'the no-way-back check needs the new look')) { return } }
    $m = Open-Options
    $items = @(if ($m) { Menu-Items $m | Where-Object { -not $_.Separator } | ForEach-Object { [string]$_.Text } })
    Close-Menus
    $look = @($items | Where-Object { $_ -match $rx.Classic -or $_ -match $rx.TryNew })
    Record 'one look' 'probe' 'the Options menu over the new look offers no other look' ($null -ne $m -and $look.Count -eq 0) $(if ($m) { "items: $($items -join ' / ')" } else { 'the Options menu did not open' })
    $t = Open-TrayMenu
    $titems = @(if ($t) { Menu-Items $t | Where-Object { -not $_.Separator } | ForEach-Object { [string]$_.Text } })
    Close-Menus
    $tlook = @($titems | Where-Object { $_ -match $rx.Classic -or $_ -match $rx.TryNew })
    Record 'one look' 'probe' 'the tray menu offers no other look' ($null -ne $t -and $tlook.Count -eq 0) $(if ($t) { "items: $($titems -join ' / ')" } else { 'the tray menu did not open' })
    if ((Look-Now) -eq 'none') { [void](Restore-Main) }
}
# The classic window is a fallback for one run: WebView2 killed twice in a
# row gives it, with new_ui=1 kept (new_ui_failed=1: the next start tries
# the new look again), and its Options menu's "Try the new look" brings the
# new look back without a restart. `Times` rounds of that.
function Fallback-Rounds([int]$Times) {
    if ((Look-Now) -ne 'new') { if (-not (Reset-App 'the fallback rounds need the new look')) { return } }
    # After a fallback the next start gives up at the first failure until its
    # page has worked a minute (new_ui_failed=1, cleared then): wait for that,
    # so that every round starts with the page's second chance.
    if ((Setting 'new_ui_failed') -eq '1') {
        $cleared = Wait-Until { (Setting 'new_ui_failed') -ne '1' } 90 1000
        Record 'fallback' 'probe' 'a page that works a minute clears new_ui_failed (the next start gets its second chance back)' ([bool]$cleared) "new_ui_failed=$(Setting 'new_ui_failed')"
        Health 'waiting for new_ui_failed to clear'
        if ($script:abort) { return }
    }
    $fell = 0; $kept = 0; $back = 0; $both = 0; $n = 0; $painted = 0
    for ($i = 1; $i -le $Times; $i++) {
        if (-not (Have-Time 1.5)) { Note 'budget' 'fallback rounds' "stopped after $n of $Times rounds (time budget)"; break }
        if ((Look-Now) -ne 'new') { Log "fallback round ${i}: the new look is not on screen ($(Look-Now))"; break }
        $n++
        # The page and its WebView2 processes must be there before they are killed.
        if (-not (Wait-Until { @(WebView-Pids).Count -gt 0 } 30 300)) { Log "fallback round ${i}: no WebView2 process to kill"; break }
        Start-Sleep -Seconds 3
        $mark = Log-Mark
        [void](Kill-WebViews)
        [void](Wait-Until { Log-Lines 'webui: .*showing the page again' $mark | Select-Object -First 1 } 30 300)
        [void](Wait-Until { @(WebView-Pids).Count -gt 0 } 30 300)
        Health "fallback round $i, WebView2 killed once"
        if ($script:abort) { return }
        Start-Sleep -Seconds 2
        [void](Kill-WebViews)
        $line = Wait-Until { Log-Lines 'webui: .*using the classic window for this run' $mark | Select-Object -First 1 } 40 300
        $onScreen = Wait-Until { if ((Look-Now) -eq 'classic') { 'classic' } } 10 200
        Health "fallback round $i, WebView2 killed twice"
        if ($script:abort) { return }
        if ((Look-Now) -eq 'both') { $both++; Shot "fallback-$i-two-windows" }
        if ($line -and $onScreen -eq 'classic') { $fell++ } else { Shot "fallback-$i-no-classic"; Log ("fallback round ${i}: on screen '$(Look-Now)'; log: " + ((Log-Lines 'webui' $mark | Select-Object -Last 5) -join ' // ')) }
        if ((Setting 'new_ui') -eq '1' -and (Setting 'new_ui_failed') -eq '1') { $kept++ } else { Log "fallback round ${i}: new_ui=$(Setting 'new_ui') new_ui_failed=$(Setting 'new_ui_failed')" }
        if ((Look-Now) -ne 'classic') { break }
        # Impatient every other time: "Try the new look" at once.
        if ($i % 2) { Start-Sleep -Milliseconds 1500 } else { Start-Sleep -Milliseconds 300 }
        $mark = Log-Mark
        [void](Options-Choose $rx.TryNew)
        if (Wait-Until { (Look-Now) -eq 'new' } 30 200) { $back++ } else { Shot "fallback-$i-not-back"; Log ("fallback round ${i}: still '$(Look-Now)' after Try the new look; log: " + ((Log-Lines 'webui' $mark | Select-Object -Last 4) -join ' // ')) }
        Health "fallback round $i, back to the new look"
        if ($script:abort) { return }
        if ((Look-Now) -eq 'both') { $both++; Shot "fallback-$i-two-windows-back" }
        Start-Sleep -Seconds 4
        $w = Shown-Main
        if ($w -and (Look-Now) -eq 'new') { $p = Painted $w; if ($p -and $p.Colours -ge 8 -and $p.Top -lt 0.97) { $painted++ } }
        if ($i -eq 1) { Shot 'fallback-and-back' }
    }
    Record 'fallback' 'probe' "WebView2 killed twice in a row gives the classic window for this run ($Times rounds)" ($n -eq $Times -and $fell -eq $n) "$fell of $n rounds"
    Record 'fallback' 'probe' 'the fallback keeps the new look for the next start (new_ui=1, new_ui_failed=1)' ($n -gt 0 -and $kept -eq $n) "$kept of $n rounds; now new_ui=$(Setting 'new_ui') new_ui_failed=$(Setting 'new_ui_failed')"
    Record 'fallback' 'probe' '"Try the new look" in the fallback''s Options menu brings the new look back without a restart' ($fell -gt 0 -and $back -eq $fell) "$back of $fell"
    Record 'fallback' 'probe' 'the page is painted again after "Try the new look"' ($back -gt 0 -and $painted -eq $back) "$painted of $back"
    Record 'fallback' 'probe' 'never both main windows on screen at once' ($both -eq 0) "$both times"
    Shot 'after-fallback-rounds'
}
function Kill-WebViews {
    $ids = @(WebView-Pids)
    foreach ($id in $ids) {
        $q = Get-Process -Id $id -ErrorAction SilentlyContinue | Where-Object ProcessName -eq 'msedgewebview2'
        if ($q) { try { $q.Kill() } catch { } }
    }
    Log "ended $($ids.Count) WebView2 process(es): $($ids -join ', ')"
    return $ids.Count
}
function WebView-Kills {
    if ((Look-Now) -ne 'new') { if (-not (Reset-App 'WebView2 kill test needs the new look')) { return } }
    if ((Look-Now) -ne 'new') { Record 'webview killed' 'probe' 'the new look is on screen before the test' $false "on screen: $(Look-Now)"; return }
    Start-Sleep -Seconds 6
    $wv = @(WebView-Pids)
    Record 'webview killed' 'probe' "the program's WebView2 processes are found (children of bahth)" ($wv.Count -gt 0) "$($wv.Count) msedgewebview2 processes under process $($script:proc.Id)"
    if (-not $wv.Count) { return }
    # Once: the page is shown again.
    $mark = Log-Mark
    [void](Kill-WebViews)
    $line = Wait-Until { Log-Lines 'webui: .*showing the page again' $mark | Select-Object -First 1 } 30 300
    Health 'WebView2 killed once'
    if ($script:abort) { return }
    $said = (Log-Lines 'webui' $mark) -join ' // '
    Record 'webview killed' 'probe' 'once: the log says the page is shown again' ($null -ne $line) $said
    $again = Wait-Until { if (@(WebView-Pids).Count) { 'yes' } } 30 300
    Record 'webview killed' 'probe' 'once: new WebView2 processes start' ([bool]$again) "$(@(WebView-Pids).Count) processes"
    Start-Sleep -Seconds 5
    $onScreen = Look-Now
    Record 'webview killed' 'probe' 'once: the new look stays on screen, alone' ($onScreen -eq 'new') "on screen: $onScreen"
    Record 'webview killed' 'probe' 'once: new_ui stays 1' ((Setting 'new_ui') -eq '1') "new_ui=$(Setting 'new_ui')"
    $w = Shown-Main
    if ($w -and $onScreen -eq 'new') { $p = Painted $w; Record 'webview killed' 'probe' 'once: the page is painted again' ($p -and $p.Colours -ge 8 -and $p.Top -lt 0.97) $(if ($p) { "$($p.Colours) colours, most common $($p.Top)" } else { 'no screenshot' }) }
    Shot 'webview-killed-once'
    if ($onScreen -ne 'new') { Note 'webview killed' 'twice in a row' "skipped: after the first kill the program is on '$onScreen' ($said)"; return }
    # Twice in a row (within the minute after which a failure is forgiven):
    # the classic window for this run only (1.0: the new look is never
    # turned off; new_ui_failed=1 makes the next start try it once more).
    $mark = Log-Mark
    [void](Kill-WebViews)
    $line = Wait-Until { Log-Lines 'webui: .*using the classic window for this run' $mark | Select-Object -First 1 } 40 300
    Health 'WebView2 killed twice in a row'
    if ($script:abort) { return }
    $said = (Log-Lines 'webui' $mark) -join ' // '
    $onScreen = Wait-Until { if ((Look-Now) -eq 'classic') { 'classic' } } 10 200
    Record 'webview killed' 'probe' 'twice: the log says the classic window is used for this run' ($null -ne $line) $said
    Record 'webview killed' 'probe' 'twice: the classic window is on screen, alone (no zombie new-look window)' ($onScreen -eq 'classic') "on screen: $(Look-Now)"
    Start-Sleep -Seconds 1
    Record 'webview killed' 'probe' 'twice: the new look is kept for the next start (new_ui=1, new_ui_failed=1)' ((Setting 'new_ui') -eq '1' -and (Setting 'new_ui_failed') -eq '1') "new_ui=$(Setting 'new_ui') new_ui_failed=$(Setting 'new_ui_failed')"
    Note 'webview killed' 'what the classic window says' (Status-Text)
    $left = Wait-Until { @(WebView-Pids).Count -eq 0 } 10 300
    Record 'webview killed' 'probe' 'twice: no WebView2 process left behind' ([bool]$left) "$(@(WebView-Pids).Count) left"
    Shot 'webview-killed-twice'
    # The next start tries the new look again.
    Keep-Log 'after the WebView2 kills'
    Stop-App 'restart after the WebView2 kills'
    $mark = Log-Mark
    $w = Start-App 60
    [void](Wait-Until { (Look-Now) -eq 'new' } 20 250)
    Start-Sleep -Seconds 2
    Record 'webview killed' 'probe' 'the next start opens the new look again (one more try)' ($w -and (Look-Now) -eq 'new') "on screen: $(Look-Now); log: $((Log-Lines 'webui' $mark | Select-Object -First 4) -join ' // ')"
    Health 'start after the WebView2 kills'
}
function Restore-Acl {
    if (-not $script:aclDenied) { return }
    $code = Run-Native 'icacls.exe' ('"{0}" /remove:d "*S-1-1-0" /T /C /Q' -f $wvDir)
    if ($code -eq 0) { $script:aclDenied = $false } else { Say "icacls could not remove the deny entry from $wvDir (exit $code)" }
}
function ReadOnly-Profile {
    Keep-Log 'before the read-only profile test'
    Stop-App 'read-only WebView2 folder'
    New-Item -ItemType Directory -Force $wvDir | Out-Null
    Write-Settings 1 0
    # Everyone: no new files or folders, no writes, no deletes inside.
    $code = Run-Native 'icacls.exe' ('"{0}" /deny "*S-1-1-0:(OI)(CI)(WD,AD,WEA,WA,DC)"' -f $wvDir)
    $script:aclDenied = ($code -eq 0)
    if (-not $script:aclDenied) { Record 'read-only profile' 'probe' 'SCRIPT: the WebView2 folder could be made read-only (icacls)' $false "exit code $code" }
    else {
        try {
            $mark = Log-Mark
            $w = Start-App 60
            Start-Sleep -Seconds 4
            Health 'start with a read-only WebView2 folder'
            $onScreen = Look-Now
            if ($onScreen -eq 'new') {
                # WebView2 started all the same; give the blank-page watchdog (20 s) its time.
                Start-Sleep -Seconds 24
                Health 'read-only WebView2 folder, after the watchdog'
                $onScreen = Look-Now
            }
            $said = (Log-Lines 'webui' $mark) -join ' // '
            $flags = "new_ui=$(Setting 'new_ui') new_ui_failed=$(Setting 'new_ui_failed')"
            Record 'read-only profile' 'probe' 'a main window appears within 60 s' ($null -ne $w) "on screen: $onScreen"
            Record 'read-only profile' 'probe' 'exactly one main window (no zombie)' ($onScreen -eq 'classic' -or $onScreen -eq 'new') "on screen: $onScreen"
            if ($onScreen -eq 'new') { Note 'read-only profile' 'first start' "WebView2 started although its folder is read-only; $flags; log: $said" }
            else { Record 'read-only profile' 'probe' 'first failed start: classic window, the new look kept for the next start (new_ui=1, new_ui_failed=1)' ($onScreen -eq 'classic' -and (Setting 'new_ui') -eq '1' -and (Setting 'new_ui_failed') -eq '1') "$flags; log: $said" }
            Shot 'read-only-profile-first-start'
            Keep-Log 'read-only profile, first start'
            Stop-App 'read-only profile, second start'
            $mark = Log-Mark
            $w = Start-App 60
            Start-Sleep -Seconds 4
            Health 'second start with a read-only WebView2 folder'
            $onScreen2 = Look-Now
            $said = (Log-Lines 'webui' $mark) -join ' // '
            $flags = "new_ui=$(Setting 'new_ui') new_ui_failed=$(Setting 'new_ui_failed')"
            if ($onScreen -ne 'new') { Record 'read-only profile' 'probe' 'second failed start in a row: the classic window again, the new look still kept (new_ui=1, new_ui_failed=1)' ($onScreen2 -eq 'classic' -and (Setting 'new_ui') -eq '1' -and (Setting 'new_ui_failed') -eq '1') "on screen: $onScreen2; $flags; log: $said" }
            else { Note 'read-only profile' 'second start' "on screen: $onScreen2; $flags; log: $said" }
            Shot 'read-only-profile-second-start'
            Keep-Log 'read-only profile, second start'
            Stop-App 'read-only profile done'
        } finally { Restore-Acl }
    }
    Record 'read-only profile' 'probe' 'SCRIPT: the folder is writable again' (-not $script:aclDenied) ''
    Write-Settings 1 0
    $w = Start-App 60
    [void](Wait-Until { (Look-Now) -eq 'new' } 20 250)
    Start-Sleep -Seconds 6
    $p = $null
    $m = Shown-Main
    if ($m) { $p = Painted $m }
    $painted = if ($p) { "$($p.Colours) colours" } else { 'no screenshot' }
    Record 'read-only profile' 'probe' 'the new look works again once the folder is writable' ((Look-Now) -eq 'new' -and $p -and $p.Colours -ge 8) "on screen: $(Look-Now); $painted"
    Health 'start after the read-only profile test'
}

function Language-Storm([int]$Times) {
    $n = 0; $ok = 0
    for ($i = 0; $i -lt $Times; $i++) {
        if (-not (Have-Time 0.5)) { Note 'budget' 'language' "stopped after $n of $Times switches (time budget)"; break }
        $want = if ($script:lang -eq 'en') { 'ar' } else { 'en' }
        $pat = if ($want -eq 'ar') { '^العربية$' } else { '^English$' }
        $old = $script:proc.Id
        $script:expectExit = $true
        $it = Options-Choose $pat $rx.Language
        if (-not $it) { $script:expectExit = $false; Record 'language' 'probe' "Options > Language > $want can be chosen" $false 'menu or item not found'; break }
        $dlg = Wait-Until { Dialogs | Select-Object -First 1 } 5 100
        if (-not $dlg) { $script:expectExit = $false; Record 'language' 'probe' 'choosing a language asks to restart' $false "no question after choosing $want (status: $(Status-Text))"; break }
        if ($i -eq 0) { Note 'language' 'the question' (Dialog-Text $dlg); Shot 'language-question' }
        [void](Dismiss-Dialogs 'yes' "restart in $want")
        $n++
        $w = Adopt-Restarted $old 45
        if (-not $w -and $script:proc -and $script:proc.Id -eq $old -and (Proc-Alive)) {
            # Still the old process: the restart never began (the answer did
            # not reach the question). Not the program's fault.
            $script:expectExit = $false
            $leftOpen = Dismiss-Dialogs 'no' 'language question left open'
            Record 'language' 'probe' "SCRIPT: answering Yes starts the restart ($want)" $false "process $old still runs after 45 s; question still open: $($leftOpen -join ' // ')"
            break
        }
        if (-not $w) {
            Record 'language' 'core' "the program comes back after its language restart ($want)" $false "no new bahth process with a window within 45 s; processes: $(@(Bahth-Procs | ForEach-Object { $_.Id }) -join ', ')"
            Shot "language-restart-$i"
            Revive "language restart $i"
            if ($script:abort) { return }
            $script:lang = (Get-ItemProperty $regKey -ErrorAction SilentlyContinue).Language
            continue
        }
        $arabicTitle = $w.Text -match '[\u0600-\u06FF]'
        $reg = (Get-ItemProperty $regKey -ErrorAction SilentlyContinue).Language
        if (($arabicTitle -eq ($want -eq 'ar')) -and $reg -eq $want) { $ok++ } else { Log "language ${i}: wanted $want, title '$($w.Text)', registry '$reg'" }
        $script:lang = $want
        if ($i -eq 0) { Start-Sleep -Seconds 2; Shot "language-$want" }
        Start-Sleep -Milliseconds 800
        Health "language switched to $want"
        if ($script:abort) { return }
        [void](Wait-Until { @(Bahth-Procs).Count -eq 1 } 15 250)
    }
    Record 'language' 'probe' "language switched $Times times; each restart came back in the chosen language" ($n -eq $Times -and $ok -eq $n) "$ok of $n right (title and registry)"
    Record 'language' 'probe' 'one process after the restarts' (@(Bahth-Procs).Count -eq 1) "$(@(Bahth-Procs).Count) bahth processes"
    if ($Look -eq 'new') { Record 'language' 'probe' 'still the new look after the restarts' ((Look-Now) -eq 'new') "on screen: $(Look-Now)" }
}

function Diag([string]$When) {
    Log "---- diagnostics: $When"
    try {
        if ($script:proc) {
            Log "process $($script:proc.Id) alive=$(Proc-Alive); on screen: $(Look-Now)"
            foreach ($w in @(App-Windows)) { Log "  window $($w.Handle) $($w.Class) visible=$($w.Visible) $($w.X),$($w.Y) $($w.W)x$($w.H) '$($w.Text)'" }
        }
        Log "  bahth processes: $(@(Bahth-Procs | ForEach-Object { $_.Id }) -join ', '); WebView2: $(@(WebView-Pids) -join ', ')"
        Log "  settings.txt: $((Read-Settings) -replace '\s+', ' ')"
        foreach ($l in @(Log-Lines '.' | Select-Object -Last 30)) { Log "  log: $l" }
    } catch { Log "diagnostics failed: $_" }
}
function New-Scratch([string]$Name, [string]$Text) {
    $p = Join-Path $scratch $Name
    [IO.File]::WriteAllText($p, $Text, $utf8)
    return $p
}

# ================================================================ the run
$script:setupFailed = $false
$wv2 = $null
try {
    $screen = [Windows.Forms.SystemInformation]::VirtualScreen
    Say ("UI stress, {0} look: PowerShell {1}, {2}, screen {3}x{4}; budget {5} min (first index: up to {6}, idle: {7}), seed {8}" -f $Look, $PSVersionTable.PSVersion, [Environment]::OSVersion.VersionString, $screen.Width, $screen.Height, $MaxMinutes, $IndexWaitMinutes, $IdleMinutes, $Seed)
    # The last resort, a few minutes after the budget and before the job's timeout.
    [S]::ArmWatchdog(($MaxMinutes + 4) * 60, $Exe, (Join-Path $Out 'watchdog.txt'), [string]$env:GITHUB_STEP_SUMMARY)
    $free = [S]::FreeDiskGb()
    Note 'setup' 'machine' ("{0}; screen {1}x{2}; {3:N1} GB free on the system drive; {4} thread" -f [Environment]::OSVersion.VersionString, $screen.Width, $screen.Height, $free, [Threading.Thread]::CurrentThread.GetApartmentState())
    if ($free -ge 0 -and $free -lt 12) {
        Write-Host ("::error::Only {0:N1} GB free on the system drive; the UI stress test wants 12 GB so that 10 GB stay free. Nothing was run." -f $free)
        $script:setupFailed = $true
    }
    $pe = [IO.File]::ReadAllBytes($Exe)
    Note 'setup' 'exe' ("{0}: {1:N0} bytes, PE machine {2:x4}" -f $Exe, $pe.Length, [BitConverter]::ToUInt16($pe, [BitConverter]::ToInt32($pe, 0x3c) + 4))
    $pe = $null

    # The WebView2 runtime the new look needs, as its installer registers it
    # (as in ci/newlook-test.ps1).
    $key = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    foreach ($k in @("HKLM:\SOFTWARE\WOW6432Node\$key", "HKLM:\SOFTWARE\$key", "HKCU:\Software\$key")) {
        $v = (Get-ItemProperty -Path $k -Name pv -ErrorAction SilentlyContinue).pv
        if ($v -and $v -ne '0.0.0.0' -and -not $wv2) { $wv2 = $v }
    }
    Note 'setup' 'WebView2 runtime' $(if ($wv2) { $wv2 } else { 'none registered' })
    if ($Look -eq 'new' -and -not $wv2) {
        Write-Host '::error::No WebView2 runtime on this runner, so the new look cannot be stress-tested here.'
        $script:setupFailed = $true
    }
    if ($script:setupFailed) { $script:abort = $true }

    if (-not $script:abort) {
        # A clean start. English first; the language scenario switches it.
        foreach ($p in @(Bahth-Procs)) { try { $p.Kill() } catch { } }
        Start-Sleep -Milliseconds 800
        Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item $regKey -Recurse -Force -ErrorAction SilentlyContinue
        New-Item $regKey -Force | Out-Null
        Set-ItemProperty $regKey -Name Language -Value 'en'
        $script:lang = 'en'

        # Synthetic files in a normal user folder, made before the program
        # starts so the first index holds them.
        Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force $scratch | Out-Null
        $script:keep = @(
            [pscustomobject]@{ What = 'Arabic name'; Q = 'تقرير ستريس'; Path = (New-Scratch 'zqk تقرير مالي ستريس.txt' 'الميزانية السنوية zqcontentar') }
            [pscustomobject]@{ What = 'English name'; Q = 'zqk budget'; Path = (New-Scratch 'zqk Stress Budget Report.txt' 'revenue forecast zqcontenten') }
            [pscustomobject]@{ What = 'mixed Arabic and English name'; Q = 'مختلط mixed'; Path = (New-Scratch 'zqk مختلط mixed ملف 2026.txt' 'mixed') }
            [pscustomobject]@{ What = 'digits in the name'; Q = 'zqk 0123456789'; Path = (New-Scratch 'zqk 0123456789 digits.txt' 'digits') }
            [pscustomobject]@{ What = 'emoji in the name'; Q = 'zqk emoji'; Path = (New-Scratch ('zqk emoji ' + (& $U 0x1F600) + ' file.txt') 'emoji') }
        )
        foreach ($i in 1..24) { [void](New-Scratch ('zqvictim-{0:D2}.txt' -f $i) "victim $i") }
        foreach ($i in 1..30) { [void](New-Scratch ('zqopen-{0:D2}.txt' -f $i) "open $i") }
        foreach ($i in 1..4) { [void](New-Scratch ('zqren-{0:D2}.txt' -f $i) "rename $i"); [void](New-Scratch ('zqdel-{0:D2}.txt' -f $i) "delete $i") }

        # What is on screen and running before the test, so that only what the
        # test opens itself is ever closed.
        $script:baseWindows = @([S]::VisibleHandles())
        $script:baseNotepads = @(Get-Process -Name notepad -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })

        $env:BAHTH_UI_TEST = '1'
        # The look (1.0 ignores new_ui=0): BAHTH_CLASSIC=1 for the classic
        # window, for every start here and the program's own restarts
        # (language), which inherit it; nothing else forces a look.
        if ($Look -eq 'classic') { $env:BAHTH_CLASSIC = '1' } else { Remove-Item Env:BAHTH_CLASSIC -ErrorAction SilentlyContinue }
        foreach ($v in 'BAHTH_WEBUI', 'BAHTH_WEBUI_FAIL') { Remove-Item "Env:$v" -ErrorAction SilentlyContinue }
        if (-not $AllowWebViewNetwork) {
            # WebView2's own background requests (component updates, pings) off.
            $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--disable-background-networking --disable-component-update --disable-sync --disable-domain-reliability --no-pings'
        }
        Write-Settings $(if ($Look -eq 'new') { 1 } else { 0 })
        Write-Rates
        [S]::StartSampler((Join-Path $Out 'memory.csv'), 5000)

        # ---------------------------------------------------------- start
        [S]::Phase = 'start'
        $script:firstStart = Get-Date
        $main = Start-App 45
        Record 'start' 'core' 'the main window appears' ($null -ne $main) $(if ($main) { "'$($main.Text)' $($main.W)x$($main.H) after {0:N1} s" -f ((Get-Date) - $script:firstStart).TotalSeconds } else { 'no main window within 45 s' })
        if (-not $main) { Diag 'no main window at the start'; Shot 'no-main-window'; $script:abort = $true }
        else {
            Start-Sleep -Seconds 1
            $said = Dismiss-Dialogs 'no' 'at the start'
            if ($said.Count) { Note 'start' 'questions asked at the start' ($said -join ' // ') }
            if ($Look -eq 'new') {
                $onScreen = Look-Now
                Record 'start' 'probe' 'starts in the new look (new_ui=1)' ($onScreen -eq 'new') "on screen: $onScreen; log: $((Log-Lines 'webui') -join ' // ')"
                Start-Sleep -Seconds 4
                $m = Shown-Main
                if ($m) { $p = Painted $m; Record 'start' 'probe' 'the page is painted' ($p -and $p.Colours -ge 8 -and $p.Top -lt 0.97) $(if ($p) { "$($p.Colours) colours, most common $($p.Top)" } else { 'no screenshot' }) }
                # Does the keyboard reach the page? Type, select all, copy.
                if (Focus-Query) {
                    Type-Query 'zqprobe'
                    Start-Sleep -Milliseconds 300
                    $back = Read-Query
                    Record 'start' 'probe' 'the keyboard reaches the page (typed text can be copied back)' ((Clean ([string]$back)).Trim() -eq 'zqprobe') "typed 'zqprobe', copied back '$back'"
                    if (Focus-Query) { Clear-Query }
                }
            } else {
                Record 'start' 'probe' 'starts in the classic look (BAHTH_CLASSIC=1)' ((Look-Now) -eq 'classic') "on screen: $(Look-Now); log: $((Log-Lines 'webui') -join ' // ')"
                $ui = Find-Controls
                Record 'start' 'probe' 'search box, result list and status bar found' ($ui -and $ui.Query -and $ui.List -and $ui.Status) ''
            }
            Shot 'first-start'
            Mem-Note 'start' | Out-Null
        }
    }

    # ---------------------------------------------------------- A: typing
    Scenario 'A typing storms' 2.5 {
        $building = -not (Test-Path $index)
        Typing-Storm 'typing' 300 '300 random queries'
        $now = -not (Test-Path $index)
        Note 'typing' 'the first index during the storm' $(if ($building -and $now) { 'still being built the whole time' } elseif ($building) { 'finished during the storm' } else { 'already saved before the storm' })
        Backspace-Storm
        Hold-Keys
        Shot 'after-typing'
    }
    Scenario 'A pastes and clears' 1.5 {
        $piece = 'تقرير مالي report 2026 '
        Paste-Test '10,000 characters' (($piece * 440).Substring(0, 10000))
        Paste-Test '200,000 characters' (($piece * 8800).Substring(0, 200000))
        $special = "line one`r`nline two`nline three`ttab`t" + [string][char]0 + 'after-nul ' + (-join $bidi) + ' تقرير ' + [string][char]0x202E + 'txt.exe' + [string][char]0x202C + "`r`n`r`n" + [string][char]0x0B + [string][char]0x0C + [string][char]0x1F + ' end'
        Paste-Test 'new lines, tabs, NUL, bidi controls' $special
        # The same control characters typed, not pasted.
        if (Focus-Query) {
            Select-Query
            [S]::Type(('ab' + [string][char]9 + (-join $bidi) + [string][char]0x1F + [string][char]1 + 'cd'), 4, 5)
            Start-Sleep -Milliseconds 300
            Health 'control characters typed'
            Clear-Query
        }
        Clear-Storm
        [void](Set-Clip 'zq')
        Mem-Note 'after typing and pasting' | Out-Null
    }

    # ---------------------------------------------------------- B: window abuse
    Scenario 'B tray: hide and restore 50 times' 2.5 { Tray-Storm 50 }
    Scenario 'B window: minimise, maximise, resize, move' 1 { Window-Storm }
    Scenario 'B quick popup 50 times' 1 { Popup-Storm 50 }
    Scenario 'B floating box clicked 30 times' 0.5 { Box-Storm 30 }
    Scenario 'B Options menu 30 times' 1 { Options-Storm 30 }
    Scenario 'B Options switches' 1.5 { Toggle-Storm }
    if ($Look -eq 'new') { Scenario 'B settings page switches (new look)' 1 { Page-Settings-Storm } }
    Scenario 'B dialogs opened and closed' 1 { Dialog-Storm }
    if (-not $script:abort -and $script:proc) { Mem-Note 'after the window abuse' | Out-Null }

    # ---------------------------------------------------------- C: the first index
    if (-not $script:abort) {
        [S]::Phase = 'C waiting for the first index'
        $deadline = $script:firstStart.AddMinutes($IndexWaitMinutes)
        $tick = Get-Date
        if (-not (Test-Path $index)) { Say ("== C waiting for the first index (minute {0:N1})" -f (Elapsed-Min)) }
        while (-not (Test-Path $index) -and (Get-Date) -lt $deadline -and (Have-Time 4) -and -not $script:abort) {
            Start-Sleep -Seconds 3
            Health 'waiting for the first index' -Quick
            if (((Get-Date) - $tick).TotalSeconds -ge 60) {
                $tick = Get-Date
                $m = Mem-Now
                Log ("index wait, minute {0:N1}: bahth private {1} MB, {2} threads; status: {3}" -f (Elapsed-Min), $m.PrivateMB, $m.Threads, $(if ((Look-Now) -eq 'classic') { Status-Text } else { '(new look)' }))
            }
        }
        $script:indexReady = Test-Path $index
        $mins = ((Get-Date) - $script:firstStart).TotalMinutes
        if ($script:indexReady) {
            $script:resultKind = 'core'
            Start-Sleep -Seconds 4
            $line = Log-Lines 'index build:' | Select-Object -Last 1
            Note 'index' 'first index' ("saved within {0:N1} min of the first start; {1}" -f $mins, $line)
        } else {
            Record 'index' 'probe' "the first index is saved within $IndexWaitMinutes minutes" $false ("not saved after {0:N1} min (a slow runner disk?): the result checks below are probes, not core" -f $mins)
        }
        Health 'first index'
        if ($script:proc) { Mem-Note 'after the first index' | Out-Null }
    }

    # ---------------------------------------------------------- D: right results
    Scenario 'D right results for files that exist' 1.5 {
        if ((Look-Now) -eq 'new') {
            # Can the page's rows be read? Only then are its results core checks.
            if (Focus-Query) {
                Type-Query 'zqk'
                Start-Sleep -Milliseconds 1500
                $paths = Page-Paths 3
                $script:pageReads = $paths.Count -gt 0
                Record 'new look' 'probe' "the page's rows can be read (Ctrl+Shift+C copies the selected row's path)" $script:pageReads "paths: $($paths -join ' | ')"
            }
        }
        foreach ($k in $script:keep) {
            [void](Search-Expect $k.Q $k.Path $script:resultKind "finds the file: $($k.What)")
            Health "search: $($k.What)" -Quick
        }
        Shot 'results'
        [void](Quick-Expect 'تقرير ستريس' 'تقرير مالي ستريس' $script:resultKind 'the popup finds the Arabic file')
    }

    if ($Look -eq 'classic') {
        Scenario 'D sorting, filters, inside documents, stop' 2 {
            if ((Look-Now) -ne 'classic') { [void](Restore-Main) }
            Classic-Filters
            $k = $script:keep[1]
            [void](Search-Expect $k.Q $k.Path $script:resultKind 'right result after the sorting and the filters')
        }
    } else {
        # ------------------------------------------------------ F: the new look only
        Scenario 'F no way back to the classic look' 0.5 { No-Way-Back }
        Scenario 'F WebView2 killed once and twice' 2 { WebView-Kills }
        Scenario 'F the one-run fallback and back 4 times' 5 { Fallback-Rounds 4 }
        Scenario 'F read-only WebView2 folder' 2.5 { ReadOnly-Profile }
        Restore-Acl
        if (-not $script:abort -and (Look-Now) -ne 'new') { [void](Reset-App 'back to the new look after the look tests') }
        Scenario 'D filters, sorting, inside documents (new look, blind)' 1.5 {
            Page-Filters
            # A key that landed elsewhere may have left a filter on: clean
            # settings and a new start, then the result must be right again.
            if (Reset-App 'after the blind filter keys') {
                $k = $script:keep[1]
                [void](Search-Expect $k.Q $k.Path $script:resultKind 'right result after the filters (after a restart with clean settings)')
            }
        }
    }

    # ---------------------------------------------------------- E: language
    Scenario 'E language switched 10 times' 2.5 { Language-Storm 10 }
    if (-not $script:abort -and $script:proc) { Mem-Note 'after the language restarts' | Out-Null }

    # ---------------------------------------------------------- D2: files change meanwhile
    Scenario 'D2 typing while files are created, renamed, deleted' 2 {
        [Churn]::Start($churnDir, 10, $Seed)
        try {
            Typing-Storm 'churn' 100 '100 queries while files change'
            $token = 'zqlive' + $script:rnd.Next(100000, 999999)
            $live = Join-Path $churnDir "$token new file.txt"
            [IO.File]::WriteAllText($live, 'made while the program runs', $utf8)
            [void](Search-Expect $token $live 'probe' 'a file created just now is found without a rescan (live index)' 30)
            foreach ($k in @($script:keep | Select-Object -First 2)) { [void](Search-Expect $k.Q $k.Path $script:resultKind "finds the file while others change: $($k.What)") }
        } finally {
            [Churn]::Stop()
            Note 'churn' 'file operations in the background' ("{0} created, {1} renamed, {2} deleted, {3} errors; {4}" -f [Churn]::Created, [Churn]::Renamed, [Churn]::Deleted, [Churn]::Errors, [Churn]::StopReason)
            Remove-Item -LiteralPath $churnDir -Recurse -Force -ErrorAction SilentlyContinue
        }
        Start-Sleep -Seconds 2
        Health 'after the file churn'
    }

    # ---------------------------------------------------------- D3: results
    Scenario 'D3 keys on results deleted or renamed meanwhile' 1.5 { Gone-Rounds }
    Scenario 'D3 rename, Recycle Bin, copy on results' 1 { Live-Actions }
    Scenario 'D3 30 files opened quickly' 1.5 { Open-Many 30 }
    Scenario 'D3 right-click menu 50 times' 1 { Context-Storm 50 }
    Scenario 'D3 popup typed into while the main window searches' 1 {
        $ticked = $false
        if (Focus-Query) {
            if ((Look-Now) -eq 'classic') {
                $ui = Find-Controls
                if ($ui -and $ui.Content) {
                    if ([S]::Msg($ui.Content.Handle, $BM_GETCHECK, 0, 0) -ne 1) { [void][S]::Post($ui.Content.Handle, $BM_CLICK, 0, 0); Start-Sleep -Milliseconds 200 }
                    $ticked = $true
                    # (The click left the keyboard on the checkbox.)
                    if (Focus-Query) {
                        Type-Query 'report'
                        [S]::Chord($VK.ENTER)
                    }
                }
            } else { Type-Query 'e' }
        }
        Start-Sleep -Milliseconds 500
        [void](Quick-Expect 'تقرير ستريس' 'تقرير مالي ستريس' $script:resultKind 'the popup finds the file while the main window searches')
        [S]::Chord($VK.CTRL, $VK.ALT, $VK.F)
        $pop = Wait-Until { Popup } 3 60
        if ($pop) {
            if ([S]::GetForegroundWindow() -ne $pop.Handle) { Focus-Window $pop.Handle }
            for ($i = 0; $i -lt 25; $i++) {
                [S]::Chord($VK.CTRL, $VK.A)
                [S]::Type((Random-Query), 8, 3)
                if ($i % 10 -eq 9) { Health "popup typing $i" -Quick }
            }
            Shot 'popup-while-searching'
        }
        [void](Close-Popup)
        if ($ticked) {
            $ui = Find-Controls
            if ($ui -and $ui.Stop) { [void][S]::Post($ui.Stop.Handle, $BM_CLICK, 0, 0); Start-Sleep -Milliseconds 300 }
            for ($i = 0; $i -lt 3 -and $ui -and $ui.Content -and [S]::Msg($ui.Content.Handle, $BM_GETCHECK, 0, 0) -ne 0; $i++) { [void][S]::Post($ui.Content.Handle, $BM_CLICK, 0, 0); Start-Sleep -Milliseconds 250 }
        }
    }

    # ---------------------------------------------------------- G: the end state
    if (-not $script:abort -and $script:proc) {
        Say ("== G end state: {0} idle minutes (minute {1:N1})" -f $IdleMinutes, (Elapsed-Min))
        try {
            Close-Menus
            [void](Close-Popup)
            [void](Dismiss-Dialogs 'no' 'before the idle minutes')
            [void](Close-Extras)
            [void](Close-Opened 'before the idle minutes')
            if (Focus-Query) { Clear-Query }
            Health 'before the idle minutes'
            [S]::Phase = 'G idle'
            $marks = @()
            $marks += (Mem-Note 'idle start')
            for ($min = 1; $min -le $IdleMinutes; $min++) {
                for ($s = 0; $s -lt 6; $s++) { Start-Sleep -Seconds 10; Health "idle minute $min" -Quick }
                $marks += (Mem-Note "idle +$min min")
            }
            Health 'after the idle minutes'
            $first = $marks[0]; $last = $marks[$marks.Count - 1]; $mid = $marks[[Math]::Max(0, $marks.Count - 3)]
            $drift = $last.PrivateMB - $mid.PrivateMB
            $grow = $last.PrivateMB - $first.PrivateMB
            $settled = ([Math]::Abs($drift) -le [Math]::Max(10, 0.05 * $last.PrivateMB)) -and ($grow -le [Math]::Max(20, 0.10 * $first.PrivateMB))
            $perMin = ($marks | ForEach-Object { $_.PrivateMB }) -join ' / '
            $wvPerMin = ($marks | ForEach-Object { $_.WvPrivateMB }) -join ' / '
            $detail = "bahth private MB each minute: {0}; last two minutes {1:+0.0;-0.0;0} MB, whole idle time {2:+0.0;-0.0;0} MB; working set {3} -> {4} MB; threads {5} -> {6}; handles {7} -> {8}; WebView2 private MB: {9}" -f $perMin, $drift, $grow, $first.WorkingMB, $last.WorkingMB, $first.Threads, $last.Threads, $first.Handles, $last.Handles, $wvPerMin
            Record 'end state' 'probe' "memory settles in $IdleMinutes idle minutes (last two minutes within 5% or 10 MB, the whole time within +10% or 20 MB)" $settled $detail
            $count = @(Bahth-Procs).Count
            Record 'end state' 'probe' 'exactly one bahth process' ($count -eq 1) "$count processes: $(@(Bahth-Procs | ForEach-Object { $_.Id }) -join ', ')"
            # A normal search still returns the right file.
            [S]::Phase = 'G last search'
            foreach ($k in @($script:keep | Select-Object -First 2)) { [void](Search-Expect $k.Q $k.Path $script:resultKind "after everything, still finds the file: $($k.What)") }
            Shot 'end-state'
            Health 'last search'
            # Exit from the tray menu.
            [S]::Phase = 'G exit'
            Keep-Log 'before the exit'
            $wv = @(WebView-Pids)
            $script:expectExit = $true
            $mark = Log-Mark
            $t = Get-Date
            $chosen = Tray-Choose $rx.TrayExit
            $gone = Wait-Until { -not @(Bahth-Procs).Count } 15 200
            # Did the program get the command? Its log says so ("exit chosen
            # from the menu"). A click that missed the item is tried once more.
            $asked = @(Log-Lines 'exit chosen from the menu' $mark).Count -gt 0
            if (-not $gone -and -not $asked) {
                Log 'the Exit item was not taken; trying once more'
                $t = Get-Date
                $chosen = Tray-Choose $rx.TrayExit
                $gone = Wait-Until { -not @(Bahth-Procs).Count } 15 200
                $asked = @(Log-Lines 'exit chosen from the menu' $mark).Count -gt 0
            }
            $secs = ((Get-Date) - $t).TotalSeconds
            if ($gone -or $asked) {
                Record 'end state' 'core' 'the program exits from the tray menu within 15 s' ([bool]$gone) ("{0:N1} s after choosing Exit; bahth processes left: {1}" -f $secs, @(Bahth-Procs).Count)
            } else {
                Record 'end state' 'probe' 'SCRIPT: Exit could be chosen from the tray menu' $false "menu and item found: $chosen; the program's log has no 'exit chosen from the menu', so the exit could not be tried"
            }
            if ($wv.Count) {
                $wvGone = Wait-Until { -not @($wv | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue | Where-Object ProcessName -eq 'msedgewebview2' }).Count } 15 300
                Record 'end state' 'probe' 'its WebView2 processes exit with it' ([bool]$wvGone) "$($wv.Count) before the exit"
            }
            if (Test-Path -LiteralPath $crash) { Health 'exit' }
            if (-not $gone) { Shot 'did-not-exit'; Diag 'did not exit' }
        } catch {
            Record 'end state' 'probe' 'SCRIPT: the end state ran to its end' $false "test script error: $($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
        }
    }
} catch {
    Record 'test' 'probe' 'SCRIPT: the test ran to its end' $false "test script error: $($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    try { Shot 'script-error'; Diag 'script error' } catch { }
} finally {
    # ------------------------------------------------------------ cleaning up
    [S]::Phase = 'cleanup'
    try { [Churn]::Stop() } catch { }
    try { Restore-Acl } catch { Log "restoring the ACL failed: $_" }
    try { Keep-Log 'end' } catch { }
    try { if (Test-Path $settingsFile) { Copy-Item $settingsFile (Join-Path $Out 'settings.txt') -Force } } catch { }
    try { if (Test-Path -LiteralPath $crash) { [IO.File]::AppendAllText((Join-Path $Out 'crash.txt'), "==== at the end`r`n" + [IO.File]::ReadAllText($crash, [Text.Encoding]::UTF8) + "`r`n", $utf8) } } catch { }
    try { Stop-App 'end of the test' } catch { Log "stopping the program failed: $_" }
    try { [S]::StopSampler() } catch { }
    try { [void](Close-Opened 'end of the test') } catch { }
    # What the test made: its files, the program's data (index, WebView2
    # profile), its registry key and its start-with-Windows entry.
    foreach ($try in 1..2) {
        Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
        if (-not (Test-Path -LiteralPath $data) -and -not (Test-Path -LiteralPath $scratch)) { break }
        Start-Sleep -Seconds 2
    }
    Remove-Item $regKey -Recurse -Force -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'BahthSari' -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' -Name 'BahthSari' -ErrorAction SilentlyContinue
    Log ("cleanup: data folder left: {0}; test files left: {1}; {2:N1} GB free" -f (Test-Path -LiteralPath $data), (Test-Path -LiteralPath $scratch), [S]::FreeDiskGb())
    try { [S]::DisarmWatchdog() } catch { }
}

# ---------------------------------------------------------------- report
# The program must not have asked the network for anything.
$kept = Join-Path $Out 'log.txt'
$net = @()
if (Test-Path $kept) { $net = @([IO.File]::ReadAllLines($kept, [Text.Encoding]::UTF8) | Where-Object { $_ -match '^\d+ (update: |exchange rates: )' } | Select-Object -Unique) }
if (-not $script:setupFailed) {
    Record 'limits' 'probe' 'no update check and no exchange-rate download by the program (its log)' ($net.Count -eq 0) (($net | Select-Object -First 4) -join ' // ')
    $refused = @()
    if (Test-Path $kept) { $refused = @([IO.File]::ReadAllLines($kept, [Text.Encoding]::UTF8) | Where-Object { $_ -match 'webui: refused a message' } | Select-Object -Unique) }
    if ($refused.Count) { Note 'new look' 'messages of the page the program refused' ("{0}: {1}" -f $refused.Count, (($refused | Select-Object -First 3) -join ' // ')) }
    Note 'stability' 'checks after steps' ("{0} checks (crash report, process, window answering), {1} failed; slowest answer {2} ms; the program was restarted by the test {3} time(s); {4} memory samples; {5} screenshots" -f $script:healthChecks, $script:healthFails, $script:maxPing, $script:revives, [S]::Samples, $script:shot)
}
Note 'budget' 'time used' ("{0:N1} of {1} minutes" -f (Elapsed-Min), $MaxMinutes)

$fail = @($results | Where-Object Result -eq 'FAIL').Count
$miss = @($results | Where-Object Result -eq 'MISS').Count
$pass = @($results | Where-Object Result -eq 'PASS').Count
$md = New-Object Text.StringBuilder
[void]$md.AppendLine("## UI stress, $Look look: $pass passed, $fail failed, $miss findings")
[void]$md.AppendLine('')
[void]$md.AppendLine(("{0:N1} minutes; first index {1}; WebView2 runtime {2}." -f (Elapsed-Min), $(if ($script:indexReady) { 'saved in time' } else { 'NOT saved in time (result checks were probes)' }), $(if ($wv2) { $wv2 } else { 'none' })))
[void]$md.AppendLine('')
[void]$md.AppendLine('FAIL = core (crash report, exit, hang over 10 s, wrong result for a file that exists); it fails the job. MISS = a finding; it does not. SCRIPT marks a problem of the test itself. Screenshots, run.log, memory.csv, log.txt and crash.txt are in the artifact.')
[void]$md.AppendLine('')
[void]$md.AppendLine('| Result | Min | Area | Check | Detail |')
[void]$md.AppendLine('|---|---|---|---|---|')
# Failures and findings first, then the notes, then what passed.
$order = @{ FAIL = 0; MISS = 1; NOTE = 2; PASS = 3 }
foreach ($r in @($results | Sort-Object { $order[$_.Result] }, Minute)) {
    $d = ($r.Detail -replace '\|', '/' -replace '[\r\n]+', ' ')
    if ($d.Length -gt 420) { $d = $d.Substring(0, 420) + '…' }
    $c = ($r.Check -replace '\|', '/')
    [void]$md.AppendLine("| $($r.Result) | $($r.Minute) | $($r.Area) | $c | $d |")
}
[void]$md.AppendLine('')
[void]$md.AppendLine('### Memory (MB)')
[void]$md.AppendLine('')
[void]$md.AppendLine('| When | Min | bahth processes | private | working set | threads | handles | WebView2 processes | WebView2 private |')
[void]$md.AppendLine('|---|---|---|---|---|---|---|---|---|')
foreach ($m in $memory) {
    [void]$md.AppendLine("| $($m.When) | $($m.Minute) | $($m.M.Procs) | $($m.M.PrivateMB) | $($m.M.WorkingMB) | $($m.M.Threads) | $($m.M.Handles) | $($m.M.WebViews) | $($m.M.WvPrivateMB) |")
}
[void]$md.AppendLine('')
[void]$md.AppendLine('Every 5 s sample (private bytes, working set, threads, handles, GDI and USER objects, per process) is in memory.csv.')
try { [IO.File]::WriteAllText((Join-Path $Out 'report.md'), $md.ToString(), (New-Object Text.UTF8Encoding $true)) } catch { Write-Host "report.md could not be written: $_" }
if ($env:GITHUB_STEP_SUMMARY) {
    try { [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $md.ToString(), (New-Object Text.UTF8Encoding $false)) } catch { Write-Host "the job summary could not be written: $_" }
}
Say ("UI stress, {0} look: {1} passed, {2} failed, {3} findings in {4:N1} minutes. Details: the job summary and the artifact." -f $Look, $pass, $fail, $miss, (Elapsed-Min))
if ($script:setupFailed) { exit 2 }
if ($fail -gt 0) { exit 1 }
