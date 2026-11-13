// Helpers of ci/torture-test.ps1, loaded there with Add-Type.
//
// Windows PowerShell 5.1 compiles this with the C# 5 compiler of .NET
// Framework: no string interpolation, no ?., no nameof, no out var, no
// expression-bodied members. Check before pushing:
//   mcs -langversion:5 -target:library -r:System.IO.Compression.dll ci/torture-test.cs
// The file is pure ASCII on purpose (\u escapes for everything else), so no
// tool can damage it by guessing an encoding.
//
//   TT  windows, messages, keystrokes, running the command line
//   TM  memory sampler, watchdog, process helpers
//   TF  files through Win32 with \\?\ paths (names PowerShell cannot make)
//   TN  the catalogue of hostile file names
//   TD  documents and archives for "search inside documents"
//   TJ  one long file job on a background thread
//   TC  churn: storms of creates, renames and deletes, and the soak mix
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.IO.Compression;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;

// ---------------------------------------------------------------------------
// Windows, messages and keystrokes (after ci/ui-test.ps1's U, with time
// limits on every message so a hung program cannot hang the test).
// ---------------------------------------------------------------------------
public static class TT {
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }
    public struct RECT { public int Left, Top, Right, Bottom; }
    delegate bool EnumProc(IntPtr h, IntPtr l);

    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode)] static extern IntPtr SmtBuf(IntPtr h, uint msg, IntPtr w, StringBuilder l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode)] static extern IntPtr SmtStr(IntPtr h, uint msg, IntPtr w, string l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW")] static extern IntPtr Smt(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "PostMessageW")] static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint item, StringBuilder sb, int n, uint flags);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr VirtualAllocEx(IntPtr p, IntPtr addr, IntPtr size, uint type, uint protect);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr p, IntPtr addr, IntPtr size, uint type);
    [DllImport("kernel32.dll")] static extern bool WriteProcessMemory(IntPtr p, IntPtr addr, byte[] buf, IntPtr n, out IntPtr written);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr p, IntPtr addr, byte[] buf, IntPtr n, out IntPtr read);

    const uint KEYUP = 2, UNICODE = 4;
    const uint SMTO_ABORTIFHUNG = 2;

    static INPUT Key(ushort vk, ushort scan, uint flags) {
        var i = new INPUT { type = 1 };
        i.u.ki = new KEYBDINPUT { wVk = vk, wScan = scan, dwFlags = flags };
        return i;
    }

    // Types text as a person would, one UTF-16 unit at a time.
    public static void Type(string text) {
        foreach (char c in text) {
            var keys = new[] { Key(0, c, UNICODE), Key(0, c, UNICODE | KEYUP) };
            SendInput(2, keys, Marshal.SizeOf(typeof(INPUT)));
            Thread.Sleep(25);
        }
    }

    // Presses a chord such as Alt+F4: modifiers down, key, all up.
    public static void Chord(params ushort[] vks) {
        var list = new List<INPUT>();
        foreach (var v in vks) list.Add(Key(v, 0, 0));
        for (int i = vks.Length - 1; i >= 0; i--) list.Add(Key(vks[i], 0, KEYUP));
        SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
    }

    public static void Click(int x, int y, bool right) {
        SetCursorPos(x, y);
        Thread.Sleep(80);
        var down = new INPUT { type = 0 }; down.u.mi = new MOUSEINPUT { dwFlags = right ? 8u : 2u };
        var up = new INPUT { type = 0 }; up.u.mi = new MOUSEINPUT { dwFlags = right ? 16u : 4u };
        SendInput(2, new[] { down, up }, Marshal.SizeOf(typeof(INPUT)));
    }

    // SendMessage with a time limit. Returns the result, or long.MinValue
    // when the window did not answer in `ms`.
    public static long Send(IntPtr h, uint msg, long w, long l, int ms) {
        IntPtr r;
        IntPtr ok = Smt(h, msg, new IntPtr(w), new IntPtr(l), SMTO_ABORTIFHUNG, (uint)ms, out r);
        return ok == IntPtr.Zero ? long.MinValue : r.ToInt64();
    }

    public static bool Post(IntPtr h, uint msg, long w, long l) {
        return PostMessageW(h, msg, new IntPtr(w), new IntPtr(l));
    }

    // How long the window's thread takes to answer WM_NULL, in ms; -1 when
    // it did not within `ms` (a hung user interface).
    public static long Ping(IntPtr h, int ms) {
        var sw = Stopwatch.StartNew();
        IntPtr r;
        IntPtr ok = Smt(h, 0 /* WM_NULL */, IntPtr.Zero, IntPtr.Zero, 0 /* SMTO_NORMAL */, (uint)ms, out r);
        return ok == IntPtr.Zero ? -1 : sw.ElapsedMilliseconds;
    }

    public static string Text(IntPtr h) {
        var sb = new StringBuilder(4096);
        IntPtr r;
        SmtBuf(h, 0x000D /* WM_GETTEXT */, (IntPtr)sb.Capacity, sb, SMTO_ABORTIFHUNG, 3000, out r);
        return sb.ToString();
    }

    // WM_SETTEXT: for a single-line edit box this also raises EN_CHANGE, so
    // the program searches as if the text had been typed. Works without the
    // keyboard focus and with text no keyboard can type (lone surrogates).
    public static bool SetText(IntPtr h, string text, int ms) {
        IntPtr r;
        return SmtStr(h, 0x000C /* WM_SETTEXT */, IntPtr.Zero, text, SMTO_ABORTIFHUNG, (uint)ms, out r) != IntPtr.Zero;
    }

    // Only these classes are asked for their text: every question costs up
    // to 3 s when the program is busy, and it has many helper windows.
    static readonly string[] TextClasses = { "NativeWindowsGuiWindow", "#32770", "Button", "Static", "Edit" };

    static string Describe(IntPtr h) {
        var c = new StringBuilder(256); GetClassName(h, c, 256);
        RECT r; GetWindowRect(h, out r);
        string text = Array.IndexOf(TextClasses, c.ToString()) >= 0 ? Text(h).Replace("|", "/").Replace("\r", " ").Replace("\n", " ") : "";
        return h.ToInt64() + "|" + c + "|" + IsWindowVisible(h) + "|" + r.Left + "|" + r.Top + "|" + (r.Right - r.Left) + "|" + (r.Bottom - r.Top) + "|" + text;
    }

    // Top-level windows: "handle|class|visible|x|y|w|h|text"; pid 0 = every
    // visible window of any process.
    public static string[] Windows(int pid) {
        var found = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (pid == 0 ? IsWindowVisible(h) : p == pid) found.Add(h);
            return true;
        }, IntPtr.Zero);
        var lines = new List<string>();
        foreach (var h in found) lines.Add(Describe(h));
        return lines.ToArray();
    }

    // Visible popup menus (#32768) of any process, without asking any other
    // window for its text.
    public static long[] MenuWindows() {
        var found = new List<long>();
        EnumWindows((h, l) => {
            if (IsWindowVisible(h)) {
                var c = new StringBuilder(64); GetClassName(h, c, 64);
                if (c.ToString() == "#32768") found.Add(h.ToInt64());
            }
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    public static int PidOf(IntPtr h) {
        uint p; GetWindowThreadProcessId(h, out p);
        return (int)p;
    }

    public static string[] Children(IntPtr parent) {
        var found = new List<IntPtr>();
        EnumChildWindows(parent, (h, l) => { found.Add(h); return true; }, IntPtr.Zero);
        var lines = new List<string>();
        foreach (var h in found) lines.Add(Describe(h));
        return lines.ToArray();
    }

    static IntPtr MenuHandle(IntPtr menuWindow) {
        IntPtr m;
        if (Smt(menuWindow, 0x01E1 /* MN_GETHMENU */, IntPtr.Zero, IntPtr.Zero, SMTO_ABORTIFHUNG, 3000, out m) == IntPtr.Zero) return IntPtr.Zero;
        return m;
    }

    // Items of a popup menu window (#32768).
    public static string[] MenuItems(IntPtr menuWindow) {
        var items = new List<string>();
        IntPtr m = MenuHandle(menuWindow);
        if (m == IntPtr.Zero) return items.ToArray();
        int n = GetMenuItemCount(m);
        for (int i = 0; i < n; i++) {
            var sb = new StringBuilder(512);
            GetMenuString(m, (uint)i, sb, 512, 0x400 /* MF_BYPOSITION */);
            if (sb.Length > 0) items.Add(sb.ToString().Replace("\t", "  "));
        }
        return items.ToArray();
    }

    // { menu handle, position } of the first item whose text matches
    // `pattern`, or null. The program's menus report a choice as
    // WM_MENUCOMMAND(position, menu handle), which the test can post itself.
    public static long[] MenuFind(IntPtr menuWindow, string pattern) {
        IntPtr m = MenuHandle(menuWindow);
        if (m == IntPtr.Zero) return null;
        var re = new Regex(pattern);
        int n = GetMenuItemCount(m);
        for (int i = 0; i < n; i++) {
            var sb = new StringBuilder(512);
            GetMenuString(m, (uint)i, sb, 512, 0x400);
            if (re.IsMatch(sb.ToString())) return new long[] { m.ToInt64(), i };
        }
        return null;
    }

    // Reads text that another process writes into memory we hand it, for list
    // views (LVM_GETITEMTEXTW) and status bars (SB_GETTEXTW).
    static string Remote(IntPtr h, Func<IntPtr, IntPtr, IntPtr, string> ask) {
        uint pid; GetWindowThreadProcessId(h, out pid);
        IntPtr proc = OpenProcess(0x0008 | 0x0010 | 0x0020 | 0x0400, false, pid);
        if (proc == IntPtr.Zero) return null;
        IntPtr mem = VirtualAllocEx(proc, IntPtr.Zero, (IntPtr)16384, 0x3000, 0x04);
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

    public static int ListCount(IntPtr list) {
        long n = Send(list, 0x1004 /* LVM_GETITEMCOUNT */, 0, 0, 5000);
        return n == long.MinValue ? -1 : (int)n;
    }

    // Row `item`, column `sub` of a list view in another 64-bit process; null
    // when the program did not answer in 5 s.
    public static string ListText(IntPtr list, int item, int sub) {
        return Remote(list, (proc, mem, h) => {
            var lv = new byte[88];
            BitConverter.GetBytes(1).CopyTo(lv, 0);                         // mask = LVIF_TEXT
            BitConverter.GetBytes(item).CopyTo(lv, 4);                      // iItem
            BitConverter.GetBytes(sub).CopyTo(lv, 8);                       // iSubItem
            BitConverter.GetBytes(mem.ToInt64() + 1024).CopyTo(lv, 24);     // pszText
            BitConverter.GetBytes(4096).CopyTo(lv, 32);                     // cchTextMax
            IntPtr w; WriteProcessMemory(proc, mem, lv, (IntPtr)lv.Length, out w);
            IntPtr r;
            if (Smt(h, 0x1073 /* LVM_GETITEMTEXTW */, (IntPtr)item, mem, SMTO_ABORTIFHUNG, 5000, out r) == IntPtr.Zero) return null;
            return ReadString(proc, mem + 1024, 8192);
        });
    }

    public static string StatusText(IntPtr bar, int part) {
        return Remote(bar, (proc, mem, h) => {
            IntPtr r;
            if (Smt(h, 0x040D /* SB_GETTEXTW */, (IntPtr)part, mem, SMTO_ABORTIFHUNG, 3000, out r) == IntPtr.Zero) return null;
            return ReadString(proc, mem, 8192);
        });
    }

    // ---- running a program and reading what it prints (UTF-8) -------------

    // (Properties, not fields: PowerShell's Sort-Object and Measure-Object
    // only see properties.)
    public class RunResult {
        public RunResult() { Out = ""; Err = ""; Line = ""; }
        public int Code { get; set; }
        public string Out { get; set; }
        public string Err { get; set; }
        public bool TimedOut { get; set; }
        public long Ms { get; set; }
        public string Line { get; set; }
    }

    // One argument quoted for a Windows command line (CommandLineToArgvW
    // rules, which Rust's std::env::args follows).
    public static string Quote(string a) {
        var sb = new StringBuilder("\"");
        int slashes = 0;
        foreach (char c in a) {
            if (c == '\\') { slashes++; continue; }
            if (c == '"') { sb.Append('\\', slashes * 2 + 1); sb.Append('"'); slashes = 0; continue; }
            sb.Append('\\', slashes); slashes = 0; sb.Append(c);
        }
        sb.Append('\\', slashes * 2);
        sb.Append('"');
        return sb.ToString();
    }

    // Runs `exe` with `args`, never longer than `timeoutMs` (it is killed
    // then: Code -999, TimedOut true).
    public static RunResult Run(string exe, string[] args, int timeoutMs) {
        var r = new RunResult();
        var line = new StringBuilder();
        foreach (var a in args) { if (line.Length > 0) line.Append(' '); line.Append(Quote(a)); }
        r.Line = line.ToString();
        var psi = new ProcessStartInfo(exe);
        psi.Arguments = r.Line;
        psi.UseShellExecute = false;
        psi.RedirectStandardOutput = true;
        psi.RedirectStandardError = true;
        psi.CreateNoWindow = true;
        psi.StandardOutputEncoding = Encoding.UTF8;
        psi.StandardErrorEncoding = Encoding.UTF8;
        var sw = Stopwatch.StartNew();
        try {
            using (var p = Process.Start(psi)) {
                var o = p.StandardOutput.ReadToEndAsync();
                var e = p.StandardError.ReadToEndAsync();
                if (!p.WaitForExit(timeoutMs)) {
                    r.TimedOut = true;
                    try { p.Kill(); } catch (Exception) { }
                    p.WaitForExit(5000);
                }
                try { if (o.Wait(5000)) r.Out = o.Result; } catch (Exception) { }
                try { if (e.Wait(5000)) r.Err = e.Result; } catch (Exception) { }
                r.Code = r.TimedOut ? -999 : p.ExitCode;
            }
        } catch (Exception ex) {
            r.Code = -998;
            r.Err = "could not start: " + ex.Message;
        }
        r.Ms = sw.ElapsedMilliseconds;
        return r;
    }
}

// ---------------------------------------------------------------------------
// Memory sampler, watchdog, process helpers.
// ---------------------------------------------------------------------------
public static class TM {
    static Timer sampler;
    static Timer dog;
    static StreamWriter csv;
    static readonly object gate = new object();
    static readonly List<int> children = new List<int>();
    static Stopwatch clock = Stopwatch.StartNew();

    // The label written with every sample (the step the test is in).
    public static volatile string Phase = "start";
    // Set by the watchdog: every loop of the test and of the generators
    // below stops as soon as it sees it.
    public static volatile bool Expired;
    // Asks the generators (TF, TC, TD) to stop early.
    public static volatile bool Stop;

    public static double Seconds() { return clock.Elapsed.TotalSeconds; }

    public static Process[] App() { return Process.GetProcessesByName("bahth"); }

    public static int AppCount() {
        var ps = App();
        int n = ps.Length;
        foreach (var p in ps) p.Dispose();
        return n;
    }

    // Kills every bahth process (and nothing else); returns how many.
    public static int KillApp() {
        int n = 0;
        foreach (var p in App()) {
            try { p.Kill(); n++; } catch (Exception) { }
            p.Dispose();
        }
        return n;
    }

    // A process the test itself started, to be stopped by the watchdog.
    public static void AddChild(int pid) { lock (children) children.Add(pid); }

    public static void KillChildren() {
        int[] pids;
        lock (children) { pids = children.ToArray(); children.Clear(); }
        foreach (int pid in pids) {
            try { using (var p = Process.GetProcessById(pid)) p.Kill(); } catch (Exception) { }
        }
    }

    // Samples private bytes, working set, threads and handles of every bahth
    // process every `periodMs` into `path` (CSV).
    public static void StartSampler(string path, int periodMs) {
        csv = new StreamWriter(path, false, new UTF8Encoding(false));
        csv.WriteLine("seconds,phase,pid,private_mb,working_set_mb,threads,handles,cpu_s");
        csv.Flush();
        sampler = new Timer(Sample, null, 0, periodMs);
    }

    static void Sample(object state) {
        if (!Monitor.TryEnter(gate)) return;
        try {
            if (csv == null) return;
            double t = Seconds();
            string phase = (Phase ?? "").Replace(',', ';');
            foreach (var p in App()) {
                try {
                    csv.WriteLine(string.Format(CultureInfo.InvariantCulture, "{0:F1},{1},{2},{3:F1},{4:F1},{5},{6},{7:F1}",
                        t, phase, p.Id, p.PrivateMemorySize64 / 1048576.0, p.WorkingSet64 / 1048576.0, p.Threads.Count, p.HandleCount, p.TotalProcessorTime.TotalSeconds));
                } catch (Exception) {
                    // The process ended between the listing and the reading.
                } finally { p.Dispose(); }
            }
            csv.Flush();
        } catch (Exception) {
        } finally { Monitor.Exit(gate); }
    }

    public static void StopSampler() {
        if (sampler != null) { sampler.Dispose(); sampler = null; }
        lock (gate) { if (csv != null) { csv.Dispose(); csv = null; } }
    }

    // After `seconds`: sets Expired and Stop, kills bahth and the test's own
    // child processes, so whatever the script waits for ends and it can
    // write its report before the job's time limit.
    public static void StartWatchdog(int seconds) {
        dog = new Timer(Bark, null, (long)seconds * 1000, Timeout.Infinite);
    }

    static void Bark(object state) {
        Expired = true;
        Stop = true;
        KillChildren();
        KillApp();
    }

    public static void StopWatchdog() {
        if (dog != null) { dog.Dispose(); dog = null; }
    }

    // Kills the program the moment `path` exists (an index save writes
    // index.tmp first). Milliseconds waited, or -1 when it never appeared.
    public static long KillWhenFileAppears(string path, int timeoutMs) {
        var sw = Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < timeoutMs && !Expired) {
            if (File.Exists(path)) { KillApp(); return sw.ElapsedMilliseconds; }
            Thread.Sleep(3);
        }
        return -1;
    }

    // Waits until `path` exists; milliseconds waited, or -1.
    public static long WaitForFile(string path, int timeoutMs) {
        var sw = Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < timeoutMs && !Expired) {
            if (File.Exists(path)) return sw.ElapsedMilliseconds;
            Thread.Sleep(3);
        }
        return -1;
    }

    public class MemStat {
        public string Phase { get; set; }
        public int Pid { get; set; }
        public int Samples { get; set; }
        public double Minutes { get; set; }
        public double FirstMB { get; set; }
        public double LastMB { get; set; }
        public double PeakMB { get; set; }
        public double SettledMB { get; set; }
        public double WsPeakMB { get; set; }
        // Least-squares slope of private bytes over the phase.
        public double SlopeMBperMin { get; set; }
        public int ThreadsFirst { get; set; }
        public int ThreadsLast { get; set; }
        public int ThreadsMax { get; set; }
        public int HandlesFirst { get; set; }
        public int HandlesLast { get; set; }
        public int HandlesMax { get; set; }
    }

    // One row for each (phase, process) of the sampler's CSV, in time order.
    public static MemStat[] Analyze(string path) {
        var order = new List<string>();
        var groups = new Dictionary<string, List<double[]>>();
        foreach (var line in File.ReadAllLines(path)) {
            var f = line.Split(',');
            double t;
            if (f.Length < 7 || !double.TryParse(f[0], NumberStyles.Float, CultureInfo.InvariantCulture, out t)) continue;
            string key = f[1] + "\u0001" + f[2];
            List<double[]> g;
            if (!groups.TryGetValue(key, out g)) { g = new List<double[]>(); groups[key] = g; order.Add(key); }
            try {
                g.Add(new[] { t, double.Parse(f[3], CultureInfo.InvariantCulture), double.Parse(f[4], CultureInfo.InvariantCulture),
                    double.Parse(f[5], CultureInfo.InvariantCulture), double.Parse(f[6], CultureInfo.InvariantCulture) });
            } catch (FormatException) { }
        }
        var stats = new List<MemStat>();
        foreach (var key in order) {
            var g = groups[key];
            if (g.Count == 0) continue;
            var k = key.Split('\u0001');
            var s = new MemStat();
            s.Phase = k[0];
            s.Pid = int.Parse(k[1], CultureInfo.InvariantCulture);
            s.Samples = g.Count;
            s.Minutes = (g[g.Count - 1][0] - g[0][0]) / 60.0;
            s.FirstMB = g[0][1];
            s.LastMB = g[g.Count - 1][1];
            s.ThreadsFirst = (int)g[0][3]; s.ThreadsLast = (int)g[g.Count - 1][3];
            s.HandlesFirst = (int)g[0][4]; s.HandlesLast = (int)g[g.Count - 1][4];
            double sx = 0, sy = 0, sxx = 0, sxy = 0;
            foreach (var r in g) {
                if (r[1] > s.PeakMB) s.PeakMB = r[1];
                if (r[2] > s.WsPeakMB) s.WsPeakMB = r[2];
                if ((int)r[3] > s.ThreadsMax) s.ThreadsMax = (int)r[3];
                if ((int)r[4] > s.HandlesMax) s.HandlesMax = (int)r[4];
                double x = r[0] / 60.0;
                sx += x; sy += r[1]; sxx += x * x; sxy += x * r[1];
            }
            double n = g.Count;
            double den = n * sxx - sx * sx;
            s.SlopeMBperMin = (g.Count >= 6 && Math.Abs(den) > 1e-9) ? (n * sxy - sx * sy) / den : 0;
            // Settled: the median of the last minute (12 samples at 5 s).
            int take = Math.Min(12, g.Count);
            var last = new List<double>();
            for (int i = g.Count - take; i < g.Count; i++) last.Add(g[i][1]);
            last.Sort();
            s.SettledMB = last[last.Count / 2];
            stats.Add(s);
        }
        return stats.ToArray();
    }
}

// ---------------------------------------------------------------------------
// Files through Win32 with \\?\ paths: long paths, trailing dots and spaces,
// reserved device names, lone surrogates, odd timestamps, sparse files.
// Nothing here throws: a false or -1 result and TF.LastError say what failed.
// ---------------------------------------------------------------------------
public static class TF {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool WriteFile(IntPtr h, byte[] buf, uint n, out uint written, IntPtr overlapped);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool CreateDirectoryW(string name, IntPtr sa);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool DeleteFileW(string name);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool RemoveDirectoryW(string name);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool MoveFileExW(string from, string to, uint flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern uint GetFileAttributesW(string name);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool SetFileAttributesW(string name, uint attributes);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetFileTime(IntPtr h, ref long created, ref long accessed, ref long written);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool CreateHardLinkW(string newName, string existing, IntPtr sa);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern byte CreateSymbolicLinkW(string link, string target, uint flags);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool DeviceIoControl(IntPtr h, uint code, IntPtr inBuf, uint inSize, IntPtr outBuf, uint outSize, out uint returned, IntPtr overlapped);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetFilePointerEx(IntPtr h, long distance, out long newPointer, uint method);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetEndOfFile(IntPtr h);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern IntPtr FindFirstFileW(string pattern, out WIN32_FIND_DATA data);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool FindNextFileW(IntPtr h, out WIN32_FIND_DATA data);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool FindClose(IntPtr h);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct WIN32_FIND_DATA {
        public uint dwFileAttributes;
        public uint ftCreationLow, ftCreationHigh, ftAccessLow, ftAccessHigh, ftWriteLow, ftWriteHigh;
        public uint nFileSizeHigh, nFileSizeLow, dwReserved0, dwReserved1;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string cFileName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 14)] public string cAlternateFileName;
    }

    static readonly IntPtr INVALID = new IntPtr(-1);
    const uint GENERIC_WRITE = 0x40000000, FILE_WRITE_ATTRIBUTES = 0x100;
    const uint CREATE_NEW = 1, CREATE_ALWAYS = 2, OPEN_EXISTING = 3;
    const uint FILE_ATTRIBUTE_NORMAL = 0x80, FILE_FLAG_BACKUP_SEMANTICS = 0x02000000;
    const uint ATTR_READONLY = 1, ATTR_DIRECTORY = 0x10, ATTR_REPARSE = 0x400, ATTR_INVALID = 0xFFFFFFFF;

    // The Win32 error of the last call that failed.
    public static int LastError;

    // The \\?\ form of a full path: Windows then takes the name as it is.
    public static string Long(string path) {
        if (path.StartsWith("\\\\?\\")) return path;
        if (path.StartsWith("\\\\")) return "\\\\?\\UNC\\" + path.Substring(2);
        return "\\\\?\\" + path;
    }

    static bool Fail() { LastError = Marshal.GetLastWin32Error(); return false; }

    public static bool Exists(string path) { return GetFileAttributesW(Long(path)) != ATTR_INVALID; }

    public static bool MkDir(string path) {
        if (CreateDirectoryW(Long(path), IntPtr.Zero)) return true;
        int e = Marshal.GetLastWin32Error();
        if (e == 183 /* ERROR_ALREADY_EXISTS */) return true;
        LastError = e;
        return false;
    }

    // Creates `path` and every missing folder above it.
    public static bool MkDirs(string path) {
        if (Exists(path)) return true;
        int cut = path.LastIndexOf('\\');
        if (cut > 2 && !MkDirs(path.Substring(0, cut))) return false;
        return MkDir(path);
    }

    // Writes a file. `replace` false fails when a file of that name (or one
    // that differs only by case, on a case-insensitive folder) exists.
    public static bool Write(string path, byte[] data, bool replace) {
        IntPtr h = CreateFileW(Long(path), GENERIC_WRITE, 0, IntPtr.Zero, replace ? CREATE_ALWAYS : CREATE_NEW, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
        if (h == INVALID) return Fail();
        bool ok = true;
        if (data != null && data.Length > 0) {
            uint done;
            ok = WriteFile(h, data, (uint)data.Length, out done, IntPtr.Zero);
            if (!ok) LastError = Marshal.GetLastWin32Error();
        }
        CloseHandle(h);
        return ok;
    }

    public static bool WriteText(string path, string text, bool replace) {
        return Write(path, new UTF8Encoding(false).GetBytes(text), replace);
    }

    public static bool Delete(string path) { return DeleteFileW(Long(path)) || Fail(); }
    public static bool RmDir(string path) { return RemoveDirectoryW(Long(path)) || Fail(); }
    public static bool Move(string from, string to) { return MoveFileExW(Long(from), Long(to), 0) || Fail(); }
    public static bool SetAttr(string path, uint attributes) { return SetFileAttributesW(Long(path), attributes) || Fail(); }
    public static bool HardLink(string newName, string existing) { return CreateHardLinkW(Long(newName), Long(existing), IntPtr.Zero) || Fail(); }

    // A symbolic link; `target` is stored as given (relative, absolute, UNC).
    public static bool SymLink(string link, string target, bool directory) {
        if (CreateSymbolicLinkW(Long(link), target, (directory ? 1u : 0u) | 2u /* unprivileged */) != 0) return true;
        if (CreateSymbolicLinkW(Long(link), target, directory ? 1u : 0u) != 0) return true;
        return Fail();
    }

    // Sets the three times of a file or folder to `fileTime` (100 ns units
    // since 1601, as Windows stores them).
    public static bool SetTimes(string path, long fileTime) {
        IntPtr h = CreateFileW(Long(path), FILE_WRITE_ATTRIBUTES, 7, IntPtr.Zero, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, IntPtr.Zero);
        if (h == INVALID) return Fail();
        long c = fileTime, a = fileTime, w = fileTime;
        bool ok = SetFileTime(h, ref c, ref a, ref w);
        if (!ok) LastError = Marshal.GetLastWin32Error();
        CloseHandle(h);
        return ok;
    }

    // A sparse file that claims `size` bytes and takes none on disk.
    public static bool Sparse(string path, long size) {
        IntPtr h = CreateFileW(Long(path), GENERIC_WRITE, 0, IntPtr.Zero, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
        if (h == INVALID) return Fail();
        uint ret; long np;
        bool ok = DeviceIoControl(h, 0x000900C4 /* FSCTL_SET_SPARSE */, IntPtr.Zero, 0, IntPtr.Zero, 0, out ret, IntPtr.Zero)
            && SetFilePointerEx(h, size, out np, 0) && SetEndOfFile(h);
        if (!ok) LastError = Marshal.GetLastWin32Error();
        CloseHandle(h);
        if (!ok) DeleteFileW(Long(path));
        return ok;
    }

    // Deletes a tree whatever its names are: never follows a junction or a
    // symbolic link (it removes the link itself), clears read-only. Returns
    // how many entries could NOT be deleted. Stops when TM.Expired is set.
    public static int DeleteTree(string path) {
        if (!Exists(path)) return 0;
        int failed = DeleteInside(Long(path));
        if (!RemoveDirectoryW(Long(path))) {
            SetFileAttributesW(Long(path), FILE_ATTRIBUTE_NORMAL);
            if (!RemoveDirectoryW(Long(path)) && !DeleteFileW(Long(path))) { LastError = Marshal.GetLastWin32Error(); failed++; }
        }
        return failed;
    }

    static int DeleteInside(string dir) {
        int failed = 0;
        var folders = new List<string>();
        WIN32_FIND_DATA d;
        IntPtr h = FindFirstFileW(dir + "\\*", out d);
        if (h == INVALID) return 0;
        try {
            do {
                if (TM.Expired) return failed + 1;
                string name = d.cFileName;
                if (name == "." || name == "..") continue;
                string full = dir + "\\" + name;
                bool isDir = (d.dwFileAttributes & ATTR_DIRECTORY) != 0;
                if ((d.dwFileAttributes & ATTR_READONLY) != 0) SetFileAttributesW(full, FILE_ATTRIBUTE_NORMAL);
                if (isDir && (d.dwFileAttributes & ATTR_REPARSE) == 0) { folders.Add(full); continue; }
                bool ok = isDir ? RemoveDirectoryW(full) : DeleteFileW(full);
                if (!ok) { LastError = Marshal.GetLastWin32Error(); failed++; }
            } while (FindNextFileW(h, out d));
        } finally { FindClose(h); }
        foreach (var f in folders) {
            failed += DeleteInside(f);
            if (!RemoveDirectoryW(f)) { LastError = Marshal.GetLastWin32Error(); failed++; }
        }
        return failed;
    }

    // `count` empty files "<prefix><first+i as 6 hex digits><suffix>" in
    // `dir`. Returns how many were created.
    public static int CreateMany(string dir, string prefix, int first, int count, string suffix) {
        string root = Long(dir);
        int made = 0;
        for (int i = 0; i < count; i++) {
            if (TM.Stop) break;
            IntPtr h = CreateFileW(root + "\\" + prefix + (first + i).ToString("x6") + suffix, GENERIC_WRITE, 0, IntPtr.Zero, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
            if (h == INVALID) { LastError = Marshal.GetLastWin32Error(); continue; }
            CloseHandle(h);
            made++;
        }
        return made;
    }

    // A chain of folders under `root` whose names are `segment` units long,
    // down to a full path of about `maxPath` units, with a small file at
    // each depth where the path length first passes one of `marks`. The file
    // names are "<token>-len<N>.txt" with tokens "zq" + 6 hex digits from
    // `firstToken` on. Returns "token|path length|full path" for each file.
    public static string[] DeepChain(string root, int segment, int maxPath, int[] marks, int firstToken) {
        var made = new List<string>();
        if (!MkDirs(root)) return made.ToArray();
        string dir = root;
        int token = firstToken;
        int mark = 0;
        int level = 0;
        while (!TM.Stop) {
            // Room for "\" + a file name of at most 40 units below this folder.
            if (dir.Length + 1 + segment + 41 > maxPath) break;
            level++;
            string tag = "d" + level.ToString("d3") + "-";
            // Arabic letters and Latin ones in turn, so the long path is not plain ASCII.
            char fill = (level % 2 == 0) ? '\u0645' : 'x';
            dir = dir + "\\" + tag + new string(fill, segment - tag.Length);
            if (!MkDir(dir)) break;
            while (mark < marks.Length && dir.Length + 30 >= marks[mark]) {
                string tok = "zq" + token.ToString("x6");
                string file = dir + "\\" + tok + "-len";
                file = file + (file.Length + 9) + ".txt";
                if (WriteText(file, "deep " + tok, true)) { made.Add(tok + "|" + file.Length + "|" + file); token++; }
                mark++;
            }
        }
        // The deepest file: its full path is exactly maxPath units long.
        if (!TM.Stop) {
            string tok = "zq" + token.ToString("x6");
            string file = dir + "\\" + tok + "-max";
            int pad = maxPath - file.Length - 4;
            if (pad >= 0 && pad < 250) {
                file = file + new string('m', pad) + ".txt";
                if (WriteText(file, "deepest " + tok, true)) made.Add(tok + "|" + file.Length + "|" + file);
            }
        }
        return made.ToArray();
    }
}

// ---------------------------------------------------------------------------
// The catalogue of hostile file names. "{T}" stands for the token the test
// searches for ("zq" + 6 hex digits), so every file can be found again
// whatever else its name holds.
// ---------------------------------------------------------------------------
public static class TN {
    // Each entry: { kind, flags, name, name... }. Names of one entry share a
    // token (names that differ only by case or by Unicode normalisation).
    // Flags: "cs" needs a case-sensitive folder; "lone" holds a lone
    // surrogate (the index leaves such names out: they only must not break
    // anything); "dev" are reserved device names, created inside a folder
    // that carries the token.
    public static List<string[]> Catalogue() {
        var c = new List<string[]>();
        Action<string, string, string[]> add = (kind, flags, names) => {
            var e = new List<string>(); e.Add(kind); e.Add(flags); e.AddRange(names); c.Add(e.ToArray());
        };
        string pad = new string('x', 300);
        // Plain and Arabic.
        add("plain", "", new[] { "plain {T}.txt" });
        add("arabic", "", new[] { "\u062A\u0642\u0631\u064A\u0631 \u0645\u0627\u0644\u064A {T}.txt" });
        add("arabic diacritics", "", new[] { "\u0627\u0644\u0652\u0639\u064E\u0631\u064E\u0628\u0650\u064A\u0651\u064E\u0629\u064F {T} \u0645\u064F\u062D\u064E\u0645\u0651\u064E\u062F\u064C.docx" });
        add("arabic tatweel", "", new[] { "\u062A\u0640\u0640\u0640\u0640\u0642\u0640\u0640\u0631\u064A\u0640\u0640\u0640\u0640\u0631 {T} \u0640\u0640\u0640\u0640.txt" });
        add("only diacritics and tatweel", "", new[] { "\u064B\u064C\u064D\u064E\u064F\u0650\u0651\u0652\u0640\u0640 {T}" });
        add("arabic-indic digits", "", new[] { "\u0661\u0662\u0663\u0664 {T} \u06F1\u06F2\u06F3.pdf" });
        add("persian and urdu letters", "", new[] { "\u067E\u0686\u0698\u06AF\u06A9\u06CC {T} \u0679\u0688\u0691\u06BA\u06D2.txt" });
        add("arabic presentation forms", "", new[] { "\uFEFB\uFEF7\uFEDF\uFEE0 {T} \uFDF2\uFDFA\uFDFB.txt" });
        add("hamza forms", "", new[] { "\u0623\u062D\u0645\u062F \u0625\u0628\u0631\u0627\u0647\u064A\u0645 \u0622\u062F\u0645 \u0624 \u0626 \u0621 {T}.txt" });
        add("mixed direction", "", new[] { "{T} \u0645\u0644\u0641 (final) v2.\u0661 - \u0646\u0633\u062E\u0629.docx" });
        add("hebrew syriac thaana nko", "", new[] { "\u05E9\u05DC\u05D5\u05DD \u0710\u0712\u0713 \u0780\u0781 \u07CA\u07CB {T}.txt" });
        // Bidi controls and overrides.
        add("bidi marks", "", new[] { "\u200E{T}\u200F \u061C mark \u200F.txt" });
        add("bidi embeddings", "", new[] { "\u202A{T}\u202C \u202Brle\u202C.txt" });
        add("bidi isolates", "", new[] { "\u2066lri\u2069 \u2067rli\u2069 \u2068fsi\u2069 {T}.txt" });
        add("RLO spoof", "", new[] { "{T} invoice\u202Efdp.exe" });
        add("LRO override", "", new[] { "\u202D{T} \u0639\u0631\u0628\u064A\u202C.txt" });
        add("unbalanced overrides", "", new[] { "\u202E\u202E\u202E\u202D\u202B {T} \u2067\u2067\u2066" });
        // Emoji and astral characters.
        add("emoji", "", new[] { "{T} \uD83D\uDE00\uD83D\uDE80\uD83C\uDF0D report.txt" });
        add("emoji zwj family", "", new[] { "\uD83D\uDC68\u200D\uD83D\uDC69\u200D\uD83D\uDC67\u200D\uD83D\uDC66 {T}.jpg" });
        add("emoji flags and skin tones", "", new[] { "\uD83C\uDDEE\uD83C\uDDF6 \uD83D\uDC4D\uD83C\uDFFD {T} 1\uFE0F\u20E3.png" });
        add("astral cjk and math", "", new[] { "\uD840\uDC00\uD869\uDED6 \uD835\uDC00\uD835\uDFCE {T}.txt" });
        add("astral rtl (adlam, arabic math)", "", new[] { "\uD83A\uDD00\uD83A\uDD01 \uD83B\uDE00\uD83B\uDE01 {T}.txt" });
        add("plane 16 private use and tags", "", new[] { "\uDBFF\uDFFD \uDB40\uDC41\uDB40\uDC42 \uDB40\uDD00 {T}.txt" });
        add("musical and variation selectors", "", new[] { "\uD834\uDD1E \u2764\uFE0F \u2764\uFE0E {T}.txt" });
        // Lone surrogates (not valid Unicode: the index leaves them out).
        add("lone high surrogate", "lone", new[] { "{T} lone\uD800.txt" });
        add("lone low surrogate", "lone", new[] { "\uDC00 {T}.txt" });
        add("high surrogate before a letter", "lone", new[] { "{T} \uD83Dx.txt" });
        add("swapped surrogate pair", "lone", new[] { "{T} \uDE00\uD83D.txt" });
        add("lone surrogate as the extension", "lone", new[] { "{T}.\uDBFF" });
        // Zero-width and invisible characters.
        add("zero width", "", new[] { "zero\u200Bwidth\u200C\u200Djoin\u2060 {T}.txt" });
        add("bom inside", "", new[] { "\uFEFFbom {T}\uFEFF.txt" });
        add("soft hyphen and cgj", "", new[] { "soft\u00ADhyphen\u034F \u180E {T}.txt" });
        add("spaces of every kind", "", new[] { "{T}\u00A0nbsp\u2003em\u3000ideographic\u2009thin.txt" });
        add("line and paragraph separators", "", new[] { "{T} line\u2028sep\u2029para.txt" });
        add("c1 controls and delete", "", new[] { "{T} \u007F\u0080\u0085\u009F.txt" });
        add("noncharacters", "", new[] { "{T} \uFFFE\uFFFF\uFDD0\uFFFD.txt" });
        add("interlinear annotation", "", new[] { "\uFFF9{T}\uFFFA\uFFFB.txt" });
        // Normalisation and case.
        add("nfc and nfd", "", new[] { "caf\u00E9 {T}.txt", "cafe\u0301 {T}.txt" });
        add("arabic composed and decomposed", "", new[] { "\u0622\u0645\u0627\u0644 {T}.txt", "\u0627\u0653\u0645\u0627\u0644 {T}.txt" });
        add("hangul syllable and jamo", "", new[] { "\uD55C\uAE00 {T}.txt", "\u1112\u1161\u11AB\u1100\u1173\u11AF {T}.txt" });
        add("compatibility forms", "", new[] { "\uFF46\uFF49\uFF4C\uFF45 {T}.txt", "file {T}.txt", "\uFB01le {T}.txt" });
        add("lam-alef ligature", "", new[] { "\uFEFB {T}.txt", "\u0644\u0627 {T}.txt" });
        add("kelvin and angstrom signs", "", new[] { "\u212A\u212B {T}.txt", "K\u00C5 {T}.txt" });
        add("case only", "cs", new[] { "Case {T}.TXT", "case {T}.txt", "CASE {T}.Txt" });
        add("turkish i", "", new[] { "\u0130stanbul {T}.txt", "\u0131stanbul {T}.txt" });
        add("sharp s", "", new[] { "stra\u00DFe {T}.txt", "strasse {T}.txt", "STRA\u1E9EE {T}.txt" });
        add("final sigma", "", new[] { "\u03C3\u03BF\u03C6\u03CC\u03C2 {T}.txt", "\u03A3\u039F\u03A6\u038C\u03A3 {T}.txt" });
        add("combining marks (zalgo)", "", new[] { "z\u0301\u0302\u0303\u0304\u0305\u0306\u0307\u0308\u0309\u030A\u030B\u030C a\u0334\u0335\u0336 {T}.txt" });
        // Trailing dots and spaces (only \\?\ can make them).
        add("trailing dot", "", new[] { "{T} trailing dot." });
        add("trailing dots", "", new[] { "{T} dots..." });
        add("trailing space", "", new[] { "{T} trailing space " });
        add("space after the extension", "", new[] { "{T}.txt " });
        add("leading space", "", new[] { " {T} leading space.txt" });
        add("leading dots", "", new[] { "...{T}" });
        add("dot file", "", new[] { ".{T}" });
        add("many extensions", "", new[] { "{T}.tar.gz.bak.old.txt.pdf.exe" });
        // Reserved device names, inside a folder that carries the token.
        add("device names", "dev", new[] { "CON", "PRN", "AUX", "NUL", "COM1", "COM9", "LPT1", "CON.txt", "nul.txt", "aux.c", "prn.log", "COM\u00B9", "CONIN$", "CONOUT$", "CLOCK$" });
        // 255 UTF-16 units.
        add("255 units ascii", "", new[] { ("{T} " + pad).Substring(0, 251 - 5) + ".txt" });
        add("255 units arabic", "", new[] { ("{T} " + new string('\u0639', 300)).Substring(0, 251 - 5) + ".txt" });
        add("255 units astral", "", new[] { "{T} " + Repeat("\uD83D\uDE00", 120) + ".txt" });
        add("255 units combining", "", new[] { "{T} a" + new string('\u0301', 240) + ".txt" });
        add("255 units of dots", "", new[] { "{T}" + new string('.', 240) + "x" });
        // Looks like query syntax. (: " | < > * ? cannot be in a Windows file
        // name, so look-alikes stand in for them.)
        add("starts with !", "", new[] { "!{T}" });
        add("starts with -", "", new[] { "-{T} minus" });
        add("ext look-alike", "", new[] { "{T} ext\uFF1Apdf size\uA789\uFF1E1.txt" });
        add("words of the syntax", "", new[] { "ext;pdf size=1 dm regex folder sys fuzzy root franco {T}" });
        add("fullwidth quotes and bar", "", new[] { "{T} \uFF02quoted\uFF02 a\uFF5Cb \uFF0A\uFF1F.txt" });
        add("regex characters", "", new[] { "{T} ^a.+z$ (1) [2] {3} \\d+.txt".Replace("\\", "\uFF3C") });
        add("percent hash at and", "", new[] { "{T} 100% #1 @home & co.txt" });
        add("quotes and ticks", "", new[] { "{T} 'single' `tick` ~tilde.txt" });
        add("commas and equals", "", new[] { "{T} a,b;c=d+e.txt" });
        add("office lock file", "", new[] { "~${T}.docx" });
        add("8.3 look-alike", "", new[] { "{T}~1.TXT" });
        add("environment variable", "", new[] { "%TEMP% %USERPROFILE% {T} $env;PATH.txt" });
        add("looks like options", "", new[] { "--help -s --count {T}" });
        add("looks like a path", "", new[] { "C\uA789\uFF3CWindows\uFF3Csystem32 {T}.txt" });
        return c;
    }

    static string Repeat(string s, int n) {
        var sb = new StringBuilder(s.Length * n);
        for (int i = 0; i < n; i++) sb.Append(s);
        return sb.ToString();
    }

    static readonly string[] Pools = {
        "\u0627\u0628\u062A\u062B\u062C\u062D\u062E\u062F\u0631\u0633\u0634\u0635\u0639\u063A\u0641\u0642\u0643\u0644\u0645\u0646\u0647\u0648\u064A\u0629\u0649\u0623\u0625\u0622\u0624\u0626",
        "\u064B\u064C\u064D\u064E\u064F\u0650\u0651\u0652\u0640\u0670",
        "\u200B\u200C\u200D\u200E\u200F\u061C\u202A\u202B\u202C\u202D\u202E\u2066\u2067\u2068\u2069\u2060\uFEFF\u00AD",
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
        " ._-!#$%&'()+,;=@[]^`{}~",
        "\u00E9\u00FC\u00F1\u00DF\u0130\u0131\u01C5\u0301\u0308\u0323\u4E2D\u6587\u65E5\u672C\uD55C\uAE00\u0E44\u0E17\u0E22\u05E9\u05DC\u0905\u0928",
        "\u0660\u0661\u0662\u0663\u06F4\u06F5\u06F6\uFF11\uFF12\u00B2\u2460",
    };
    static readonly string[] Astral = { "\uD83D\uDE00", "\uD83D\uDE80", "\uD83C\uDF0D", "\uD83D\uDC68\u200D\uD83D\uDCBB", "\uD840\uDC00", "\uD835\uDC00", "\uD83A\uDD00", "\uDBFF\uDFFD", "\uD83C\uDDEE\uD83C\uDDF6" };
    static readonly string[] Exts = { ".txt", ".docx", ".pdf", ".xlsx", ".jpg", ".zip", "", ".", ".tar.gz", ".\u0645\u0644\u0641" };

    // A random name of mixed scripts, marks and symbols with `token` in it.
    // The same seed always gives the same name.
    public static string RandomName(int seed, string token) {
        var rng = new Random(seed);
        var sb = new StringBuilder();
        int parts = 2 + rng.Next(10);
        int at = rng.Next(parts);
        for (int i = 0; i < parts; i++) {
            if (i == at) sb.Append(' ').Append(token).Append(' ');
            if (rng.Next(8) == 0) { sb.Append(Astral[rng.Next(Astral.Length)]); continue; }
            string pool = Pools[rng.Next(Pools.Length)];
            int n = 1 + rng.Next(8);
            for (int k = 0; k < n; k++) sb.Append(pool[rng.Next(pool.Length)]);
        }
        string name = sb.ToString().Trim();
        string ext = Exts[rng.Next(Exts.Length)];
        if (name.Length + ext.Length > 250) name = token + " cut";
        // A name may not end with a space or a dot for ordinary tools; these
        // go through \\?\, so one in ten keeps such an ending.
        if (rng.Next(10) != 0) name = name.TrimEnd(' ', '.') + "x";
        return name + ext;
    }
}

// ---------------------------------------------------------------------------
// Documents and archives for "search inside documents", made here so the
// runner needs no Office: minimal valid files, bombs and traps. Everything
// is synthetic. The bombs are small on disk and are only ever opened by the
// program under test.
// ---------------------------------------------------------------------------
public static class TD {
    static readonly UTF8Encoding Utf8 = new UTF8Encoding(false);
    static byte[] B(string s) { return Utf8.GetBytes(s); }

    static void Entry(ZipArchive zip, string name, byte[] data, CompressionLevel level) {
        var e = zip.CreateEntry(name, level);
        using (var s = e.Open()) s.Write(data, 0, data.Length);
    }

    static void ZeroEntry(ZipArchive zip, string name, long size) {
        var e = zip.CreateEntry(name, CompressionLevel.Optimal);
        var block = new byte[1 << 20];
        using (var s = e.Open()) {
            for (long done = 0; done < size && !TM.Stop; done += block.Length) s.Write(block, 0, (int)Math.Min(block.Length, size - done));
        }
    }

    static string X(string s) { return s.Replace("&", "&amp;").Replace("<", "&lt;").Replace(">", "&gt;"); }

    const string ContentTypes = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>{0}</Types>";
    const string Rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"{0}\"/></Relationships>";

    static string DocumentXml(string text) {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t xml:space=\"preserve\">" + X(text) + "</w:t></w:r></w:p></w:body></w:document>";
    }

    // A minimal Word document with one paragraph.
    public static void Docx(string path, string text) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) {
            Entry(zip, "[Content_Types].xml", B(string.Format(ContentTypes, "<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>")), CompressionLevel.Optimal);
            Entry(zip, "_rels/.rels", B(string.Format(Rels, "word/document.xml")), CompressionLevel.Optimal);
            Entry(zip, "word/document.xml", B(DocumentXml(text)), CompressionLevel.Optimal);
        }
    }

    // A minimal PowerPoint file with one slide.
    public static void Pptx(string path, string text) {
        string slide = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>" + X(text) + "</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>";
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) {
            Entry(zip, "[Content_Types].xml", B(string.Format(ContentTypes, "<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>")), CompressionLevel.Optimal);
            Entry(zip, "_rels/.rels", B(string.Format(Rels, "ppt/presentation.xml")), CompressionLevel.Optimal);
            Entry(zip, "ppt/presentation.xml", B("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"/>"), CompressionLevel.Optimal);
            Entry(zip, "ppt/slides/slide1.xml", B(slide), CompressionLevel.Optimal);
        }
    }

    static void XlsxParts(ZipArchive zip, string sheetXml, string sharedStrings) {
        Entry(zip, "[Content_Types].xml", B(string.Format(ContentTypes, "<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>")), CompressionLevel.Optimal);
        Entry(zip, "_rels/.rels", B(string.Format(Rels, "xl/workbook.xml")), CompressionLevel.Optimal);
        Entry(zip, "xl/workbook.xml", B("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"), CompressionLevel.Optimal);
        Entry(zip, "xl/_rels/workbook.xml.rels", B("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"sharedStrings.xml\"/></Relationships>"), CompressionLevel.Optimal);
        Entry(zip, "xl/sharedStrings.xml", B(sharedStrings), CompressionLevel.Optimal);
        Entry(zip, "xl/worksheets/sheet1.xml", B(sheetXml), CompressionLevel.Optimal);
    }

    static string SharedStrings(string text) {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" count=\"1\" uniqueCount=\"1\"><si><t>" + X(text) + "</t></si></sst>";
    }

    const string SheetHead = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">";

    // A minimal Excel workbook with the text in cell A1.
    public static void Xlsx(string path, string text) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create))
            XlsxParts(zip, SheetHead + "<dimension ref=\"A1\"/><sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row></sheetData></worksheet>", SharedStrings(text));
    }

    // A workbook with one cell at A1 and one at XFD1048576: a reader that
    // lays the sheet out as a dense grid would ask for hundreds of GB.
    public static void XlsxCorners(string path, string text) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create))
            XlsxParts(zip, SheetHead + "<dimension ref=\"A1:XFD1048576\"/><sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row><row r=\"1048576\"><c r=\"XFD1048576\" t=\"s\"><v>0</v></c></row></sheetData></worksheet>", SharedStrings(text));
    }

    // A workbook whose shared strings are `size` bytes of zeros.
    public static void XlsxBomb(string path, long size) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) {
            XlsxParts(zip, SheetHead + "<sheetData/></worksheet>", SharedStrings("x"));
            ZeroEntry(zip, "xl/sharedStrings2.xml", size);
        }
    }

    // A minimal OpenDocument text file.
    public static void Odt(string path, string text) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) {
            Entry(zip, "mimetype", B("application/vnd.oasis.opendocument.text"), CompressionLevel.NoCompression);
            Entry(zip, "content.xml", B("<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>" + X(text) + "</text:p></office:text></office:body></office:document-content>"), CompressionLevel.Optimal);
        }
    }

    // A one-page PDF with `text` (ASCII) in Helvetica.
    public static byte[] PdfBytes(string text) {
        string esc = text.Replace("\\", "\\\\").Replace("(", "\\(").Replace(")", "\\)");
        string stream = "BT /F1 12 Tf 72 720 Td (" + esc + ") Tj ET";
        var objs = new[] {
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
            "<< /Length " + stream.Length + " >>\nstream\n" + stream + "\nendstream",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        };
        var sb = new StringBuilder("%PDF-1.4\n");
        var offsets = new List<int>();
        for (int i = 0; i < objs.Length; i++) {
            offsets.Add(sb.Length);
            sb.Append(i + 1).Append(" 0 obj\n").Append(objs[i]).Append("\nendobj\n");
        }
        int xref = sb.Length;
        sb.Append("xref\n0 ").Append(objs.Length + 1).Append("\n0000000000 65535 f \n");
        foreach (int o in offsets) sb.Append(o.ToString("d10")).Append(" 00000 n \n");
        sb.Append("trailer\n<< /Size ").Append(objs.Length + 1).Append(" /Root 1 0 R >>\nstartxref\n").Append(xref).Append("\n%%EOF\n");
        return Encoding.ASCII.GetBytes(sb.ToString());
    }

    public static void Pdf(string path, string text) { File.WriteAllBytes(path, PdfBytes(text)); }

    public static void Rtf(string path, string text) {
        File.WriteAllBytes(path, Encoding.ASCII.GetBytes("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Arial;}}\\f0\\fs24 " + text + "\\par}"));
    }

    public static void Eml(string path, string text) {
        File.WriteAllBytes(path, Encoding.ASCII.GetBytes("From: sender@example.com\r\nTo: reader@example.com\r\nSubject: torture test\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=us-ascii\r\n\r\n" + text + "\r\n"));
    }

    // A zip with one text member.
    public static void ZipOfText(string path, string member, string text) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) Entry(zip, member, B(text), CompressionLevel.Optimal);
    }

    // A zip (or an Office container, by its name) with one member of `size`
    // zero bytes: about a thousandth of that on disk.
    public static void ZeroZip(string path, string member, long size) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) ZeroEntry(zip, member, size);
    }

    // A zip of `count` tiny text members.
    public static void ManyMembersZip(string path, int count, string text) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) {
            for (int i = 0; i < count && !TM.Stop; i++) Entry(zip, "m" + (i / 100) + "/member" + i + ".txt", B(text + " " + i), CompressionLevel.Fastest);
        }
    }

    // zip in zip in zip..., `depth` levels, with `size` zero bytes and a
    // small text member at the bottom.
    public static void NestedZip(string path, int depth, long size, string text) {
        byte[] inner;
        using (var m = new MemoryStream()) {
            using (var zip = new ZipArchive(m, ZipArchiveMode.Create, true)) {
                Entry(zip, "bottom.txt", B(text), CompressionLevel.Optimal);
                ZeroEntry(zip, "zeros.txt", size);
            }
            inner = m.ToArray();
        }
        for (int i = 1; i < depth; i++) {
            using (var m = new MemoryStream()) {
                using (var zip = new ZipArchive(m, ZipArchiveMode.Create, true)) {
                    Entry(zip, "level" + i + ".zip", inner, CompressionLevel.Optimal);
                    Entry(zip, "note" + i + ".txt", B("level " + i), CompressionLevel.Optimal);
                }
                inner = m.ToArray();
            }
        }
        File.WriteAllBytes(path, inner);
    }

    // Member names that try to leave the folder. The program only reads
    // archives into memory; nothing may appear on disk.
    public static readonly string[] TraversalNames = {
        "..\\..\\escaped-{T}.txt", "../../escaped-{T}.txt", "..\\escaped-{T}.txt", "../escaped-{T}.txt",
        "C:\\ifx-escaped-{T}.txt", "/ifx-escaped-{T}.txt", "\\\\?\\C:\\ifx-escaped-{T}.txt", "....//....//escaped-{T}.txt",
        "a/../../../escaped-{T}.txt", "CON", "nul.txt", "a:b.txt", "dir/", "", " ", "a\u0000b.txt", "x/./y/../z.txt",
    };

    public static void TraversalZip(string path, string token, string text) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) {
            foreach (var n in TraversalNames) {
                try { Entry(zip, n.Replace("{T}", token), B(text), CompressionLevel.Optimal); } catch (Exception) { }
            }
            Entry(zip, "inside.txt", B(text), CompressionLevel.Optimal);
        }
    }

    // A member name of `components` folders (a 20-unit name each).
    public static void DeepNameZip(string path, int components, string text) {
        var sb = new StringBuilder();
        for (int i = 0; i < components; i++) sb.Append("folder-").Append(i.ToString("d12")).Append('/');
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) {
            try { Entry(zip, sb + "deep.txt", B(text), CompressionLevel.Optimal); } catch (Exception) { }
            Entry(zip, "top.txt", B(text), CompressionLevel.Optimal);
        }
    }

    static void TarHeader(Stream s, string name, long size, char type) {
        var h = new byte[512];
        var nb = B(name);
        Array.Copy(nb, 0, h, 0, Math.Min(nb.Length, 100));
        Action<int, int, string> field = (at, len, text) => { var b = Encoding.ASCII.GetBytes(text); Array.Copy(b, 0, h, at, Math.Min(b.Length, len)); };
        field(100, 8, "0000644\0");
        field(108, 8, "0000000\0");
        field(116, 8, "0000000\0");
        field(124, 12, Convert.ToString(size, 8).PadLeft(11, '0') + "\0");
        field(136, 12, "00000000000\0");
        for (int i = 148; i < 156; i++) h[i] = 32;
        h[156] = (byte)type;
        field(257, 8, "ustar\0" + "00");
        int sum = 0;
        foreach (byte b in h) sum += b;
        field(148, 8, Convert.ToString(sum, 8).PadLeft(6, '0') + "\0 ");
        s.Write(h, 0, 512);
    }

    static void TarMember(Stream s, string name, byte[] data) {
        TarHeader(s, name, data.Length, '0');
        s.Write(data, 0, data.Length);
        int padding = (512 - data.Length % 512) % 512;
        s.Write(new byte[padding], 0, padding);
    }

    // A .tar.gz with the given members ("{T}" in a name becomes `token`).
    public static void TarGz(string path, string[] names, string token, string text) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var gz = new GZipStream(f, CompressionLevel.Optimal)) {
            foreach (var n in names) TarMember(gz, n.Replace("{T}", token), B(text));
            gz.Write(new byte[1024], 0, 1024);
        }
    }

    // A .tar.gz with one member of `size` zero bytes.
    public static void ZeroTarGz(string path, long size) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var gz = new GZipStream(f, CompressionLevel.Optimal)) {
            TarHeader(gz, "zeros.txt", size, '0');
            var block = new byte[1 << 20];
            for (long done = 0; done < size && !TM.Stop; done += block.Length) gz.Write(block, 0, (int)Math.Min(block.Length, size - done));
            gz.Write(new byte[1024 + 512], 0, 1024 + 512);
        }
    }

    // A .gz of plain bytes.
    public static void Gz(string path, byte[] data) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var gz = new GZipStream(f, CompressionLevel.Optimal)) gz.Write(data, 0, data.Length);
    }

    // A .gz of `size` zero bytes.
    public static void ZeroGz(string path, long size) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var gz = new GZipStream(f, CompressionLevel.Optimal)) {
            var block = new byte[1 << 20];
            for (long done = 0; done < size && !TM.Stop; done += block.Length) gz.Write(block, 0, (int)Math.Min(block.Length, size - done));
        }
    }

    // "Billion laughs": nested XML entities.
    public static string XmlBomb(string token) {
        var sb = new StringBuilder("<?xml version=\"1.0\"?>\n<!DOCTYPE lolz [\n <!ENTITY lol \"lol" + token + "\">\n");
        string prev = "lol";
        for (int i = 1; i <= 9; i++) {
            sb.Append(" <!ENTITY lol").Append(i).Append(" \"");
            for (int k = 0; k < 10; k++) sb.Append('&').Append(prev).Append(';');
            sb.Append("\">\n");
            prev = "lol" + i;
        }
        sb.Append("]>\n<lolz>&lol9;</lolz>\n");
        return sb.ToString();
    }

    // A Word document whose document.xml is the entity bomb.
    public static void DocxXmlBomb(string path, string token) {
        using (var f = new FileStream(path, FileMode.Create))
        using (var zip = new ZipArchive(f, ZipArchiveMode.Create)) {
            Entry(zip, "[Content_Types].xml", B(string.Format(ContentTypes, "")), CompressionLevel.Optimal);
            Entry(zip, "_rels/.rels", B(string.Format(Rels, "word/document.xml")), CompressionLevel.Optimal);
            Entry(zip, "word/document.xml", B(XmlBomb(token).Replace("<lolz>&lol9;</lolz>", "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>&lol9;</w:t></w:r></w:p></w:body></w:document>")), CompressionLevel.Optimal);
        }
    }

    // A text file of `size` bytes in lines of `lineLength` (0: one single
    // line), with `startToken` in the first line and `endToken` at the end.
    public static void BigText(string path, long size, int lineLength, string startToken, string endToken) {
        var head = B(startToken + " ");
        var tail = B(" " + endToken + "\n");
        var block = new byte[1 << 20];
        for (int i = 0; i < block.Length; i++) {
            block[i] = (byte)('a' + i % 23);
            if (i % 7 == 6) block[i] = 32;
            if (lineLength > 0 && i % lineLength == lineLength - 1) block[i] = 10;
        }
        using (var f = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.None, 1 << 20)) {
            f.Write(head, 0, head.Length);
            long body = size - head.Length - tail.Length;
            for (long done = 0; done < body && !TM.Stop; done += block.Length) f.Write(block, 0, (int)Math.Min(block.Length, body - done));
            f.Write(tail, 0, tail.Length);
        }
    }

    public static byte[] RandomBytes(int size, int seed) {
        var b = new byte[size];
        new Random(seed).NextBytes(b);
        return b;
    }

    // The first `length` bytes of `source`.
    public static void Truncated(string source, string target, long length) {
        var all = File.ReadAllBytes(source);
        var cut = new byte[Math.Max(0, Math.Min(all.Length, length))];
        Array.Copy(all, cut, cut.Length);
        File.WriteAllBytes(target, cut);
    }

    // The header of an OLE compound file (what an encrypted Office document
    // and the old .doc/.xls/.msg formats start with), then random bytes.
    public static void CfbStub(string path, int size, int seed) {
        var b = RandomBytes(Math.Max(size, 512), seed);
        var magic = new byte[] { 0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1 };
        Array.Copy(magic, b, magic.Length);
        // Minor and major version, byte order, sector shifts: as a real file.
        var rest = new byte[] { 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x3E, 0, 3, 0, 0xFE, 0xFF, 9, 0, 6, 0 };
        Array.Copy(rest, 0, b, 8, rest.Length);
        File.WriteAllBytes(path, b);
    }

    // `count` small text documents "doc-zq<hex6>.txt" in folders of 500, each
    // holding the unique word "zc<hex6>" (same number) among filler words.
    public static int SmallDocs(string dir, int first, int count) {
        int made = 0;
        string[] filler = { "report", "budget", "\u062A\u0642\u0631\u064A\u0631", "\u0645\u064A\u0632\u0627\u0646\u064A\u0629", "meeting", "notes", "\u0645\u0644\u0627\u062D\u0638\u0627\u062A", "invoice" };
        for (int i = 0; i < count && !TM.Stop; i++) {
            int n = first + i;
            string sub = Path.Combine(dir, "s" + (i / 500).ToString("d2"));
            if (i % 500 == 0) Directory.CreateDirectory(sub);
            var sb = new StringBuilder();
            for (int k = 0; k < 40; k++) sb.Append(filler[(n + k) % filler.Length]).Append(k % 9 == 8 ? "\r\n" : " ");
            sb.Append("zc").Append(n.ToString("x6")).Append(" end\r\n");
            File.WriteAllText(Path.Combine(sub, "doc-zq" + n.ToString("x6") + ".txt"), sb.ToString(), Utf8);
            made++;
        }
        return made;
    }
}

// ---------------------------------------------------------------------------
// One long file operation at a time on a background thread, so the script
// can keep watching the program (alive, answering) while it runs.
// ---------------------------------------------------------------------------
public static class TJ {
    static Thread thread;
    public static volatile bool Busy;
    public static volatile int Result;
    public static volatile string Error = "";
    public static double Seconds;

    // op: create, rename, delete, replace, upper (TC, numbers in `n` as their
    // parameters after `dir`), flip (a, b, times, pause), many (a = folder,
    // b = prefix, first, count), deltree (a).
    public static void Start(string op, string a, string b, int[] n) {
        Busy = true; Result = 0; Error = ""; Seconds = 0;
        thread = new Thread(() => {
            var sw = Stopwatch.StartNew();
            try { Result = Do(op, a, b, n); }
            catch (Exception e) { Error = e.GetType().Name + ": " + e.Message; }
            finally { Seconds = sw.Elapsed.TotalSeconds; Busy = false; }
        });
        thread.IsBackground = true;
        thread.Start();
    }

    static int Do(string op, string a, string b, int[] n) {
        switch (op) {
            case "create": return TC.Create(a, n[0], n[1], n[2]);
            case "rename": return TC.Rename(a, n[0], n[1], n[2], n[3], n[4], n[5]);
            case "delete": return TC.Delete(a, n[0], n[1], n[2], n[3], n[4]);
            case "replace": return TC.Replace(a, n[0], n[1], n[2], n[3], n[4]);
            case "upper": return TC.UpperCase(a, n[0], n[1], n[2], n[3], n[4]);
            case "flip": return TC.Flip(a, b, n[0], n[1]);
            case "many": return TF.CreateMany(a, b, n[0], n[1], ".dat");
            case "deltree": return TF.DeleteTree(a);
            default: throw new ArgumentException("unknown job " + op);
        }
    }
}

// ---------------------------------------------------------------------------
// Churn. File "i" of a storm is "<dir>\d<i / perDir as 3 hex>\zq<base + i as
// 6 hex>-c.txt", so the script can tell what must exist afterwards from the
// numbers alone.
// ---------------------------------------------------------------------------
public static class TC {
    static readonly byte[] Small = Encoding.ASCII.GetBytes("churn\r\n");

    public static string Tok(int n) { return "zq" + n.ToString("x6"); }

    static string Folder(string dir, int i, int perDir) { return dir + "\\d" + (i / perDir).ToString("x3"); }

    public static string PathOf(string dir, int tokenBase, int i, int perDir) {
        return Folder(dir, i, perDir) + "\\" + Tok(tokenBase + i) + "-c.txt";
    }

    // Creates files 0..count-1. Returns how many file operations succeeded.
    public static int Create(string dir, int tokenBase, int count, int perDir) {
        int ok = 0;
        for (int i = 0; i < count && !TM.Stop; i++) {
            if (i % perDir == 0) TF.MkDirs(Folder(dir, i, perDir));
            if (TF.Write(PathOf(dir, tokenBase, i, perDir), Small, true)) ok++;
        }
        return ok;
    }

    // Renames every file with i % step == phase to the token newBase + i.
    public static int Rename(string dir, int tokenBase, int count, int perDir, int step, int phase, int newBase) {
        int ok = 0;
        for (int i = phase; i < count && !TM.Stop; i += step) {
            if (TF.Move(PathOf(dir, tokenBase, i, perDir), PathOf(dir, newBase, i, perDir))) ok++;
        }
        return ok;
    }

    // Deletes every file with i % step == phase.
    public static int Delete(string dir, int tokenBase, int count, int perDir, int step, int phase) {
        int ok = 0;
        for (int i = phase; i < count && !TM.Stop; i += step) {
            if (TF.Delete(PathOf(dir, tokenBase, i, perDir))) ok++;
        }
        return ok;
    }

    // Replaces every file with i % step == phase by a new file of the same
    // name (deleted, then created with other content).
    public static int Replace(string dir, int tokenBase, int count, int perDir, int step, int phase) {
        int ok = 0;
        var other = Encoding.ASCII.GetBytes("replaced by a file of the same name\r\n");
        for (int i = phase; i < count && !TM.Stop; i += step) {
            string p = PathOf(dir, tokenBase, i, perDir);
            if (TF.Delete(p) && TF.Write(p, other, true)) ok++;
        }
        return ok;
    }

    // Renames every file with i % step == phase to the same name in capitals.
    public static int UpperCase(string dir, int tokenBase, int count, int perDir, int step, int phase) {
        int ok = 0;
        for (int i = phase; i < count && !TM.Stop; i += step) {
            string p = PathOf(dir, tokenBase, i, perDir);
            int cut = p.LastIndexOf('\\');
            if (TF.Move(p, p.Substring(0, cut + 1) + p.Substring(cut + 1).ToUpperInvariant())) ok++;
        }
        return ok;
    }

    // Renames the folder `a` to `b` and back, `times` renames in all (an odd
    // number leaves it as `b`). Returns how many renames succeeded.
    public static int Flip(string a, string b, int times, int pauseMs) {
        int ok = 0;
        for (int i = 0; i < times && !TM.Stop; i++) {
            bool done = (i % 2 == 0) ? TF.Move(a, b) : TF.Move(b, a);
            // The watcher or an antivirus may hold the folder for a moment.
            for (int retry = 0; !done && retry < 20; retry++) {
                Thread.Sleep(100);
                done = (i % 2 == 0) ? TF.Move(a, b) : TF.Move(b, a);
            }
            if (!done) break;
            ok++;
            if (pauseMs > 0) Thread.Sleep(pauseMs);
        }
        return ok;
    }

    // ---- the soak mix: a slow, endless trickle on a background thread ------

    static Thread soak;
    static volatile bool soakStop;
    static readonly object soakGate = new object();
    static readonly List<int> live = new List<int>();
    static readonly List<int> gone = new List<int>();
    static int soakNext;
    public static int SoakOps;
    public static int SoakErrors;

    // Starts creating, renaming, rewriting and deleting files under `dir`,
    // about `opsPerSecond` operations a second, tokens from `tokenBase` on.
    public static void StartSoak(string dir, int tokenBase, int opsPerSecond, int seed) {
        soakStop = false;
        soakNext = 0;
        soak = new Thread(() => {
            var rng = new Random(seed);
            int pause = Math.Max(1, 1000 / Math.Max(1, opsPerSecond));
            while (!soakStop && !TM.Stop) {
                try { SoakStep(dir, tokenBase, rng); } catch (Exception) { Interlocked.Increment(ref SoakErrors); }
                Thread.Sleep(pause);
            }
        });
        soak.IsBackground = true;
        soak.Start();
    }

    static void SoakStep(string dir, int tokenBase, Random rng) {
        int roll = rng.Next(100);
        lock (soakGate) {
            if (live.Count < 50 || roll < 40) {
                int i = soakNext++;
                if (i % 200 == 0) TF.MkDirs(Folder(dir, i, 200));
                if (TF.Write(PathOf(dir, tokenBase, i, 200), Small, true)) live.Add(i); else SoakErrors++;
            } else if (roll < 60) {
                int at = rng.Next(live.Count);
                int i = live[at];
                if (TF.Delete(PathOf(dir, tokenBase, i, 200))) { live.RemoveAt(at); gone.Add(i); } else SoakErrors++;
            } else if (roll < 80) {
                int at = rng.Next(live.Count);
                int from = live[at];
                int to = soakNext++;
                if (to % 200 == 0) TF.MkDirs(Folder(dir, to, 200));
                if (TF.Move(PathOf(dir, tokenBase, from, 200), PathOf(dir, tokenBase, to, 200))) { live.RemoveAt(at); gone.Add(from); live.Add(to); } else SoakErrors++;
            } else {
                int i = live[rng.Next(live.Count)];
                var data = new byte[64 + rng.Next(4000)];
                for (int k = 0; k < data.Length; k++) data[k] = (byte)('a' + k % 26);
                if (!TF.Write(PathOf(dir, tokenBase, i, 200), data, true)) SoakErrors++;
            }
            SoakOps++;
        }
    }

    public static void StopSoak() {
        soakStop = true;
        if (soak != null) { soak.Join(10000); soak = null; }
    }

    // Up to `n` token numbers (without the base) of files that exist now
    // (`present`) or that were deleted or renamed away.
    public static int[] SoakSample(bool present, int n, int seed) {
        lock (soakGate) {
            var from = new List<int>(present ? live : gone);
            var rng = new Random(seed);
            var pick = new List<int>();
            while (pick.Count < n && from.Count > 0) {
                int at = rng.Next(from.Count);
                pick.Add(from[at]);
                from.RemoveAt(at);
            }
            return pick.ToArray();
        }
    }

    public static int SoakLive() { lock (soakGate) return live.Count; }
}
