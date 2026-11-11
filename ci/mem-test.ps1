# Memory measurement of InstaFind (bahth.exe) on a real Windows machine, for
# up to three exes side by side: the RELEASED 0.9.1 (the last mingw build)
# and 0.10.0 (the first MSVC static-CRT build), and one built from the
# checked-out branch (-ExeBranch, shown as "branch").
#
# It is a measurement, not a gate: high memory only raises `::warning::`
# lines (settled private bytes over 1 GB, or two bahth processes alive for
# more than 60 s). The script exits 1 only when the harness itself breaks, or
# the program crashes (crash.txt) or exits by itself.
#
# Scenarios (-Scenarios, any subset; each starts from an empty data folder):
#   A  Baseline: the machine as it is. Wait for the name index, 3 min idle.
#   B  Heavy tree: a synthetic Unity-like Library/ and node_modules tree
#      (about 500,000 small files), multi-GB text files (.txt .json .log
#      .csv), a 1 GB zip, tar.gz archives, 1 GB .blend/.fbx/.asset/
#      .unitypackage dummies, and files just under the content indexer's own
#      limits. Wait for the name index, then for background content indexing
#      to settle (log.txt says so; else a time box).
#   C  Search stress, in the same process as B: 20 rounds of typed searches
#      that match almost everything and long or odd ones, a search inside
#      documents every 5th round, the Ctrl+Alt+F popup 20 times, 5 min idle.
#   E  Long idle, same process: 10 min in the tray, slope of private bytes.
#   F  Churn: started as Windows starts it (--startup, in the tray), with
#      B's 500,000 small files on disk so the index is big. After the name
#      index and 2 min of idle baseline, 15 min of continuous file churn
#      from several threads (render-like frames, Unity-Library-like small
#      files, growing downloads with chunk files, documents rewritten)
#      while saving index.bin is made slow the way a busy PC does it: every
#      core kept busy at normal priority (the program saves on a
#      below-normal thread) and the disk of the data folder kept busy with
#      unbuffered sequential writes; then 10 min quiet. Private bytes
#      against the idle baseline of the same index and against the number
#      of file operations; the program's threads by name (index-save...),
#      and every save of index.bin seen from outside (index.tmp).
#   H  The same churn with the saves delayed by the branch build's test
#      hook instead (environment variable -SaveDelayVar = -SaveDelayMs).
#   O  Watch-buffer overflow: bursts of 150,000 files created and deleted in
#      seconds; whole-drive rescans per burst (log.txt), memory afterwards.
#   K  Killed in the middle of saving index.bin, 20 times: every next start
#      must load the index (no rebuild, no crash report).
#   M  Handover with a save pending: a copy run from Downloads moves itself
#      to its permanent place and restarts (--moved); one process 15 s
#      later, no index rebuild. (0.10.0 and later.)
#   S  A second start while the first copy is suspended must come back
#      within 5 s.
#   Q  Windows session end (WM_QUERYENDSESSION, WM_ENDSESSION): the program
#      exits within 5 s.
# F, H, O, K, M, S, Q report the pass criteria the review council set for
# the branch build; the released exes are measured the same way. A FAIL of
# the branch build is a warning, and fails the run only with -FailOnCriteria.
#   D  Update path: 0.9.1 "installed" in %LOCALAPPDATA%\Programs\InstaFind
#      checks for updates from the Options menu and updates itself to the
#      latest release (D1); the other version is started while one runs
#      (D3a, D3b); and the updater's own swap + `--updated` start is
#      replayed while the old copy stays alive (D2).
#   G  The new look (WebView2), 0.10.0 and later, 3 min idle: bahth plus
#      its msedgewebview2 processes.
# B, C and E share one run of the program (B -> C -> E), as on a real PC.
#
# Every 5 s a background thread writes one line per bahth process to
# samples.csv: PID, parent PID, start time, command line, private bytes,
# private working set (what Task Manager shows), working set, peaks, handles,
# threads (and how many of each name), GDI/USER objects, CPU time, the
# msedgewebview2 total, and the churn counters.
#
# Output in <Out>: report.md (also the job summary), samples.csv,
# summary.csv, churn.csv, logs\ (log.txt of every run, kept across its
# rotation), shots\, diagnostics.txt, crash-*.txt.
# `-Merge <dir>` builds one report from the summary files of several runs.
#
# All scenarios for every exe take hours in one go; the workflow
# (.github/workflows/mem-test.yml) splits them over jobs with -Versions and
# -Scenarios, as ci/test-plan.json says.
#
# Limits it keeps: -BudgetMinutes shortens the waits that do not fit, skips
# what is left when the time is used up, and a watchdog ends the script six
# minutes later if it hangs; the synthetic data shrinks so that at least
# 12 GB stay free (the churn stops under 10 GB) and is deleted at the end;
# the only network use is the program's own update check and download in
# scenario D.
#
# Runs under Windows PowerShell 5.1 on 64-bit Windows with the x64 exes.
# This file is UTF-8 with a BOM so Windows PowerShell reads the Arabic in it.
param(
    [string]$Exe091,
    [string]$Exe0100,
    # An exe built from the checked-out branch; its version name is "branch".
    [string]$ExeBranch,
    [string]$Out = (Join-Path $env:RUNNER_TEMP 'mem-test'),
    # Which of 0.9.1, 0.10.0, branch to measure (default: every exe given).
    [string]$Versions = '',
    [string]$Scenarios = 'A,B,C,D,E,F,G,H,O,K,M,S,Q',
    # A name for this run in the report (the workflow's matrix leg).
    [string]$Leg = '',
    # Where the synthetic files go; default: the fixed drive with most room.
    [string]$DataRoot = '',
    # The time this run may take, in minutes (0: no limit). Waits that do not
    # fit are shortened, scenarios that are left are skipped, and the report
    # says so.
    [int]$BudgetMinutes = 0,
    # Multiplies every wait and the number of search rounds (0.05 for a quick
    # run of the harness itself); waits for an event keep a sensible minimum.
    [double]$TimeScale = 1.0,
    # Multiplies the size of the synthetic data.
    [double]$SizeScale = 1.0,
    # Leave the synthetic files on disk (a CI machine is thrown away).
    [switch]$KeepData,
    # Threads that keep the data folder's disk busy during F's churn (0: none).
    [int]$DiskLoadThreads = 2,
    # Normal-priority threads that keep the CPU busy during F's churn
    # (-1: one per logical processor, 0: none).
    [int]$CpuBurners = -1,
    # The branch build's test hook that delays every save of index.bin
    # (scenario H, and K): the environment variable and its value.
    [string]$SaveDelayVar = 'BAHTH_TEST_SAVE_DELAY_MS',
    [int]$SaveDelayMs = 90000,
    # Exit 1 when the branch build fails one of the council's criteria.
    [switch]$FailOnCriteria,
    # Build one report from the runs found under this folder, and stop.
    [string]$Merge = ''
)
$ErrorActionPreference = 'Stop'
$script:t0 = Get-Date
$utf8 = New-Object Text.UTF8Encoding $false

function Say([string]$Text) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $Text) }
function One-Line([string]$Text) { return (($Text -replace '[\r\n]+', ' ') -replace '\s{2,}', ' ').Trim() }

# ---------------------------------------------------------------- report tables
function Version-Key([string]$V) { try { return [version]$V } catch { return [version]'999.0' } }
function Num($V, [string]$Format = 'N0') {
    if ($null -eq $V -or "$V" -eq '' -or "$V" -eq 'NaN') { return 'n/a' }
    try { return ([double]$V).ToString($Format, [Globalization.CultureInfo]::InvariantCulture) } catch { return "$V" }
}
function Build-Table($Rows) {
    $sb = New-Object Text.StringBuilder
    [void]$sb.AppendLine('| Scenario | Version | bahth processes (max / end) | Peak private MB | OS peak private MB | Settled private MB | Settled private WS MB | Settled working set MB | Handles | Threads | GDI | WebView2 (n / MB) | Detail |')
    [void]$sb.AppendLine('|---|---|---|---|---|---|---|---|---|---|---|---|---|')
    foreach ($r in @($Rows | Sort-Object @{ Expression = { [int]$_.Order } }, @{ Expression = { Version-Key $_.Version } })) {
        $wv = if ([int]$r.Wv2Count -gt 0) { "$($r.Wv2Count) / $(Num $r.Wv2PrivateMB)" } else { '-' }
        $d = (One-Line "$($r.Detail)") -replace '\|', '/'
        [void]$sb.AppendLine("| $($r.Scenario) | $($r.Version) | $($r.ProcMax) / $($r.ProcEnd) | $(Num $r.PeakPrivateMB) | $(Num $r.OsPeakPrivateMB) | $(Num $r.SettledPrivateMB) | $(Num $r.SettledPrivWsMB) | $(Num $r.SettledWsMB) | $(Num $r.Handles) | $(Num $r.Threads) | $(Num $r.Gdi) | $wv | $d |")
    }
    return $sb.ToString()
}
function Build-ChurnTable($Rows) {
    $sb = New-Object Text.StringBuilder
    [void]$sb.AppendLine('| Scenario | Version | Idle baseline MB | Max x baseline | 10 min after x baseline | Churn s | File operations | Ops/s | Written MB | Disk load MB | Private MB before | Max during churn | At churn end | 10 min after | Growth MB per million ops | Fit MB per million ops | Kept after the quiet time MB | Threads max | index-save threads max | Saves of index.bin seen | Longest save s | Drive rescans | Generator errors |')
    [void]$sb.AppendLine('|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|')
    foreach ($r in @($Rows | Sort-Object @{ Expression = { "$($_.Scenario)" } }, @{ Expression = { Version-Key $_.Version } })) {
        [void]$sb.AppendLine("| $($r.Scenario): $($r.How) | $($r.Version) | $(Num $r.PrivateBefore) | $(Num $r.RatioMax 'N2') | $(Num $r.RatioAfter 'N2') | $(Num $r.ChurnSeconds) | $(Num $r.Ops) | $(Num $r.OpsPerSec) | $(Num $r.WrittenMB) | $(Num $r.LoadMB) | $(Num $r.PrivateBefore) | $(Num $r.PrivateMaxChurn) | $(Num $r.PrivateEndChurn) | $(Num $r.PrivateAfter) | $(Num $r.GrowthPerMillion 'N1') | $(Num $r.FitPerMillion 'N1') | $(Num $r.KeptMB) | $(Num $r.ThreadsMax) | $($r.IndexSaveThreadsMax) | $($r.SavesSeen) | $(Num $r.LongestSaveS) | $($r.Rescans) | $($r.Errors) |")
    }
    return $sb.ToString()
}
# The council's pass criteria, one column per exe.
function Build-Criteria($Rows) {
    $cols = @(@($Rows | ForEach-Object { $_.Version } | Sort-Object -Unique) | Sort-Object { Version-Key $_ })
    if (-not $cols.Count) { return '' }
    $sb = New-Object Text.StringBuilder
    [void]$sb.AppendLine("| Scenario | Criterion | $($cols -join ' | ') |")
    [void]$sb.AppendLine('|---|---|' + ('---|' * $cols.Count))
    foreach ($g in @($Rows | Group-Object { "$($_.Scenario) | $($_.Criterion)" } | Sort-Object Name)) {
        $cells = foreach ($c in $cols) {
            $r = $g.Group | Where-Object { $_.Version -eq $c } | Select-Object -First 1
            if ($r) { "**$($r.Result)** $((One-Line "$($r.Value)") -replace '\|', '/')" } else { '-' }
        }
        [void]$sb.AppendLine("| $($g.Name) | $(@($cells) -join ' | ') |")
    }
    return $sb.ToString()
}
# The exes side by side: one column per exe, the main numbers of each scenario.
function Build-Compare($Rows) {
    $cols = @(@($Rows | ForEach-Object { $_.Version } | Sort-Object -Unique) | Where-Object { @('0.9.1', '0.10.0', 'branch') -contains $_ } | Sort-Object { Version-Key $_ })
    if (-not $cols.Count) { return '' }
    $sb = New-Object Text.StringBuilder
    [void]$sb.AppendLine("| Scenario | Number | $($cols -join ' | ') |")
    [void]$sb.AppendLine('|---|---|' + ('---|' * $cols.Count))
    $metrics = @(@('Peak private MB', 'PeakPrivateMB'), @('Settled private MB', 'SettledPrivateMB'), @('Settled private WS MB (Task Manager)', 'SettledPrivWsMB'), @('Settled working set MB', 'SettledWsMB'), @('Threads', 'Threads'), @('bahth processes (max / end)', ''))
    foreach ($g in @($Rows | Where-Object { $cols -contains $_.Version } | Group-Object Scenario | Sort-Object { [int]$_.Group[0].Order })) {
        $first = $true
        foreach ($m in $metrics) {
            $cells = foreach ($c in $cols) {
                $r = $g.Group | Where-Object { $_.Version -eq $c } | Select-Object -First 1
                if (-not $r) { '-' } elseif ($m[1]) { Num $r.($m[1]) } else { "$($r.ProcMax) / $($r.ProcEnd)" }
            }
            $name = if ($first) { $g.Name } else { '' }
            [void]$sb.AppendLine("| $name | $($m[0]) | $(@($cells) -join ' | ') |")
            $first = $false
        }
    }
    return $sb.ToString()
}
$tableLegend = @'
Private = private bytes (commit), summed over all bahth processes. Private WS = private working set, the "Memory" column of Task Manager. Peak = highest 5 s sample of the scenario; OS peak = the highest private bytes Windows recorded for a process since it started (catches spikes between samples; B, C and E share one process). Settled = median of the last 30 s of the scenario.
'@

# ---------------------------------------------------------------- merge mode
if ($Merge) {
    if (-not (Test-Path -LiteralPath $Merge)) { Write-Host "::error::mem-test: nothing to merge, $Merge does not exist (no leg uploaded its results)"; exit 1 }
    $Merge = (Resolve-Path $Merge).Path
    New-Item -ItemType Directory -Force $Out | Out-Null
    $rows = @(); $churn = @()
    $md = New-Object Text.StringBuilder
    [void]$md.AppendLine('## InstaFind memory test on real Windows: 0.9.1, 0.10.0 and this branch')
    [void]$md.AppendLine('')
    # A "quick" run only shakes the harness down (short waits, small data):
    # its numbers stay out of the tables.
    $isQuick = { param($f) (Split-Path (Split-Path $f.FullName -Parent) -Leaf) -match 'quick' }
    $legs = @(Get-ChildItem -LiteralPath $Merge -Recurse -Filter 'summary.csv' -ErrorAction SilentlyContinue | Sort-Object FullName)
    foreach ($f in $legs) { if (-not (& $isQuick $f)) { $rows += @(Import-Csv -LiteralPath $f.FullName) } }
    foreach ($f in @(Get-ChildItem -LiteralPath $Merge -Recurse -Filter 'churn.csv' -ErrorAction SilentlyContinue)) { if (-not (& $isQuick $f)) { $churn += @(Import-Csv -LiteralPath $f.FullName) } }
    Say "merge: $($legs.Count) runs, $($rows.Count) rows, $($churn.Count) churn rows under $Merge"
    if ($rows.Count) {
        [void]$md.AppendLine('### Side by side')
        [void]$md.AppendLine('')
        [void]$md.AppendLine((Build-Compare $rows))
        [void]$md.AppendLine('### Every scenario')
        [void]$md.AppendLine('')
        [void]$md.AppendLine((Build-Table $rows))
        [void]$md.AppendLine($tableLegend)
    } else {
        [void]$md.AppendLine('No results: no summary.csv was found. See the logs of the `mem` jobs.')
    }
    $crit = @()
    foreach ($f in @(Get-ChildItem -LiteralPath $Merge -Recurse -Filter 'criteria.csv' -ErrorAction SilentlyContinue)) { if (-not (& $isQuick $f)) { $crit += @(Import-Csv -LiteralPath $f.FullName) } }
    if ($crit.Count) {
        [void]$md.AppendLine('### Pass criteria (set for the branch build; the released exes are measured the same way)')
        [void]$md.AppendLine('')
        [void]$md.AppendLine((Build-Criteria $crit))
    }
    if ($churn.Count) {
        [void]$md.AppendLine('### F, H. Churn: private bytes against the idle baseline and the file operations')
        [void]$md.AppendLine('')
        [void]$md.AppendLine((Build-ChurnTable $churn))
    }
    foreach ($kind in @(@('warnings.txt', 'Warnings'), @('crashes.txt', 'Crashes'), @('problems.txt', 'Harness problems'), @('notes.txt', 'Notes'))) {
        $lines = @()
        foreach ($f in @(Get-ChildItem -LiteralPath $Merge -Recurse -Filter $kind[0] -ErrorAction SilentlyContinue | Sort-Object FullName)) {
            $name = Split-Path (Split-Path $f.FullName -Parent) -Leaf
            foreach ($l in @([IO.File]::ReadAllLines($f.FullName, [Text.Encoding]::UTF8) | Where-Object { $_ })) { $lines += "- ``$name``: $l" }
        }
        if ($lines.Count) {
            [void]$md.AppendLine("### $($kind[1])")
            [void]$md.AppendLine('')
            foreach ($l in $lines) { [void]$md.AppendLine($l) }
            [void]$md.AppendLine('')
        }
    }
    foreach ($f in @(Get-ChildItem -LiteralPath $Merge -Recurse -Filter 'details.md' -ErrorAction SilentlyContinue | Sort-Object FullName)) {
        $name = Split-Path (Split-Path $f.FullName -Parent) -Leaf
        [void]$md.AppendLine("<details><summary>Details of $name</summary>")
        [void]$md.AppendLine('')
        [void]$md.AppendLine([IO.File]::ReadAllText($f.FullName, [Text.Encoding]::UTF8))
        [void]$md.AppendLine('</details>')
        [void]$md.AppendLine('')
    }
    [IO.File]::WriteAllText((Join-Path $Out 'report.md'), $md.ToString(), $utf8)
    if ($env:GITHUB_STEP_SUMMARY) {
        # The job summary takes 1 MB at most.
        $text = $md.ToString()
        if ($text.Length -gt 900000) { $text = $text.Substring(0, 900000) + "`n`n(cut: the full report is in the mem-test artifact)`n" }
        [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $text, $utf8)
    }
    # The log gets the tables; the details are in the summary and the artifact.
    $text = $md.ToString()
    $cut = $text.IndexOf('<details>')
    Write-Host $(if ($cut -gt 0) { $text.Substring(0, $cut) } else { $text })
    if (-not $legs.Count) { Write-Host '::error::mem-test: nothing to merge'; exit 1 }
    exit 0
}

# ---------------------------------------------------------------- compiled helpers
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
# Windows, keys, mouse, menus: as ci/ui-test.ps1, with a time limit on every
# message sent to the program (it may be busy for seconds under load).
Add-Type -IgnoreWarnings -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class MemUi {
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }
    public struct RECT { public int Left, Top, Right, Bottom; }
    delegate bool EnumProc(IntPtr h, IntPtr l);

    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode)] static extern IntPtr SendTimeoutText(IntPtr h, uint msg, IntPtr w, StringBuilder l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode)] static extern IntPtr SendTimeoutString(IntPtr h, uint msg, IntPtr w, string l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW")] static extern IntPtr SendTimeoutPtr(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint item, StringBuilder sb, int n, uint flags);
    [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint item, out RECT r);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr VirtualAllocEx(IntPtr p, IntPtr addr, IntPtr size, uint type, uint protect);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr p, IntPtr addr, IntPtr size, uint type);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr p, IntPtr addr, byte[] buf, IntPtr n, out IntPtr read);

    const uint KEYUP = 2, UNICODE = 4, ABORTIFHUNG = 2;
    const uint WAIT_MS = 3000;

    // SendMessage with a time limit; zero when the window did not answer.
    public static IntPtr Send(IntPtr h, uint msg, IntPtr w, IntPtr l) {
        IntPtr r;
        if (SendTimeoutPtr(h, msg, w, l, ABORTIFHUNG, WAIT_MS, out r) == IntPtr.Zero) return IntPtr.Zero;
        return r;
    }

    static INPUT Key(ushort vk, ushort scan, uint flags) {
        var i = new INPUT { type = 1 };
        i.u.ki = new KEYBDINPUT { wVk = vk, wScan = scan, dwFlags = flags };
        return i;
    }

    // Types text as a person would, one character at a time.
    public static void Type(string text, int delayMs) {
        foreach (char c in text) {
            var keys = new[] { Key(0, c, UNICODE), Key(0, c, UNICODE | KEYUP) };
            SendInput(2, keys, Marshal.SizeOf(typeof(INPUT)));
            if (delayMs > 0) System.Threading.Thread.Sleep(delayMs);
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
        IntPtr r;
        SendTimeoutText(h, 0x000D /* WM_GETTEXT */, (IntPtr)sb.Capacity, sb, ABORTIFHUNG, 1000, out r);
        return sb.ToString();
    }

    // Replaces the text of an edit box in one go (a paste).
    public static bool SetText(IntPtr h, string text) {
        IntPtr r;
        return SendTimeoutString(h, 0x000C /* WM_SETTEXT */, IntPtr.Zero, text, ABORTIFHUNG, WAIT_MS, out r) != IntPtr.Zero;
    }

    // (The text only of windows on screen: asking costs up to a second
    // each while the program is busy.)
    static string Describe(IntPtr h) {
        var c = new StringBuilder(256); GetClassName(h, c, 256);
        RECT r; GetWindowRect(h, out r);
        bool visible = IsWindowVisible(h);
        string text = visible ? Text(h).Replace("|", "/").Replace("\r", " ").Replace("\n", " ") : "";
        return h.ToInt64() + "|" + c + "|" + visible + "|" + r.Left + "|" + r.Top + "|" + (r.Right - r.Left) + "|" + (r.Bottom - r.Top) + "|" + text;
    }

    // Top-level windows of a process: "handle|class|visible|x|y|w|h|text"; pid 0 = every visible window.
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

    // The HMENU of a popup menu window (#32768).
    public static IntPtr MenuHandle(IntPtr menuWindow) {
        return Send(menuWindow, 0x01E1 /* MN_GETHMENU */, IntPtr.Zero, IntPtr.Zero);
    }

    // Every position of the menu, separators as empty strings, so an index
    // here is the item's position.
    public static string[] MenuItems(IntPtr menuWindow) {
        var items = new List<string>();
        IntPtr m = MenuHandle(menuWindow);
        if (m == IntPtr.Zero) return items.ToArray();
        int n = GetMenuItemCount(m);
        for (int i = 0; i < n; i++) {
            var sb = new StringBuilder(512);
            GetMenuString(m, (uint)i, sb, 512, 0x400 /* MF_BYPOSITION */);
            items.Add(sb.ToString().Replace("\t", "  "));
        }
        return items.ToArray();
    }

    // Screen rectangle (left, top, right, bottom) of an item of an open popup menu, or null.
    public static int[] MenuItemRect(IntPtr menuWindow, int index) {
        IntPtr m = MenuHandle(menuWindow);
        RECT r;
        if (m == IntPtr.Zero || !GetMenuItemRect(IntPtr.Zero, m, (uint)index, out r)) return null;
        if (r.Right <= r.Left || r.Bottom <= r.Top) return null;
        return new[] { r.Left, r.Top, r.Right, r.Bottom };
    }

    public static int ListCount(IntPtr list) {
        return (int)Send(list, 0x1004 /* LVM_GETITEMCOUNT */, IntPtr.Zero, IntPtr.Zero).ToInt64();
    }

    // One part of a status bar in another 64-bit process (SB_GETTEXTW into
    // memory we hand it).
    public static string StatusText(IntPtr bar, int part) {
        uint pid; GetWindowThreadProcessId(bar, out pid);
        IntPtr proc = OpenProcess(0x0008 | 0x0010 | 0x0020 | 0x0400, false, pid);
        if (proc == IntPtr.Zero) return "";
        IntPtr mem = VirtualAllocEx(proc, IntPtr.Zero, (IntPtr)8192, 0x3000, 0x04);
        try {
            if (mem == IntPtr.Zero) return "";
            Send(bar, 0x040D /* SB_GETTEXTW */, (IntPtr)part, mem);
            var buf = new byte[4096]; IntPtr read;
            ReadProcessMemory(proc, mem, buf, (IntPtr)buf.Length, out read);
            string s = Encoding.Unicode.GetString(buf);
            int z = s.IndexOf('\0');
            return z >= 0 ? s.Substring(0, z) : s;
        }
        finally { if (mem != IntPtr.Zero) VirtualFreeEx(proc, mem, IntPtr.Zero, 0x8000); CloseHandle(proc); }
    }
}
'@

# The sampler (a background thread writing samples.csv) and the generators of
# synthetic files: the static heavy tree of scenario B and the churn of F.
# Only System.dll and mscorlib are used (the zip and tar files are written by
# hand), so Add-Type needs no extra reference.
Add-Type -IgnoreWarnings -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.IO.Compression;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

public static class MemSampler {
    [StructLayout(LayoutKind.Sequential)]
    public struct Counters {
        public uint cb; public uint PageFaultCount;
        public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage, QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage, PrivateUsage, PrivateWorkingSetSize;
        public ulong SharedCommitUsage;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct MemoryStatus { public uint dwLength; public uint dwMemoryLoad; public ulong ullTotalPhys, ullAvailPhys, ullTotalPageFile, ullAvailPageFile, ullTotalVirtual, ullAvailVirtual, ullAvailExtendedVirtual; }

    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("psapi.dll", SetLastError = true)] static extern bool GetProcessMemoryInfo(IntPtr h, ref Counters c, uint cb);
    [DllImport("kernel32.dll")] static extern bool GlobalMemoryStatusEx(ref MemoryStatus m);
    [DllImport("user32.dll")] static extern uint GetGuiResources(IntPtr h, uint flags);
    [DllImport("ntdll.dll")] static extern int NtQueryInformationProcess(IntPtr h, int cls, IntPtr buf, int len, out int ret);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool QueryFullProcessImageName(IntPtr h, uint flags, StringBuilder name, ref int size);
    [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint access, bool inherit, uint tid);
    [DllImport("kernel32.dll")] static extern int GetThreadDescription(IntPtr h, out IntPtr text);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);

    public const string Header = "n,time,elapsed_s,version,scenario,phase,bahth_count,pid,parent_pid,start_time,private_mb,private_ws_mb,ws_mb,peak_ws_mb,peak_private_mb,handles,threads,gdi,user,cpu_s,wv2_count,wv2_private_mb,wv2_ws_mb,churn_ops,churn_mb,sys_avail_mb,sys_commit_mb,exe,cmdline,thread_names";

    public static volatile string Version = "", Scenario = "", Phase = "";
    public static long Samples;
    public static volatile string LastError = "";
    // The last sample, for progress lines.
    public static volatile string LastLine = "(no sample yet)";
    public static volatile int LastCount;
    public static double LastPrivateMb;

    static readonly CultureInfo Inv = CultureInfo.InvariantCulture;
    static readonly object Gate = new object();
    static readonly Stopwatch Clock = new Stopwatch();
    static readonly Dictionary<string, string[]> Known = new Dictionary<string, string[]>();
    static readonly StringBuilder Pending = new StringBuilder();
    static Thread worker;
    static volatile bool stop;
    static string file;

    public static void Start(string csvPath, int intervalMs) {
        file = csvPath;
        File.WriteAllText(file, Header + "\r\n", new UTF8Encoding(false));
        Clock.Start();
        stop = false;
        worker = new Thread(() => {
            while (!stop) {
                SampleNow();
                for (int i = 0; i < intervalMs / 100 && !stop; i++) Thread.Sleep(100);
            }
        });
        worker.IsBackground = true;
        worker.Name = "mem-sampler";
        // Keeps its 5 s step while scenario F keeps every core busy.
        worker.Priority = ThreadPriority.Highest;
        worker.Start();
    }

    public static void Stop() {
        stop = true;
        if (worker != null) worker.Join(5000);
        SampleNow();
    }

    [DllImport("ntdll.dll")] static extern int NtSuspendProcess(IntPtr h);
    [DllImport("ntdll.dll")] static extern int NtResumeProcess(IntPtr h);

    // Suspends every thread of a process (as a debugger or a frozen app), or lets it run again.
    public static bool Suspend(int pid, bool suspend) {
        IntPtr h = OpenProcess(0x0800 /* PROCESS_SUSPEND_RESUME */, false, pid);
        if (h == IntPtr.Zero) return false;
        try { return (suspend ? NtSuspendProcess(h) : NtResumeProcess(h)) == 0; }
        finally { CloseHandle(h); }
    }

    // The time budget is used up (set by the watchdog).
    public static volatile bool TimeUp;

    // Last resort against a hang: `seconds` from now the generators are told
    // to stop and TimeUp is set; if the script is still running `grace`
    // seconds after that, bahth is ended and this process exits with 124
    // (before the job's own timeout, so what was measured is still uploaded).
    public static void StartWatchdog(int seconds, int grace) {
        var t = new Thread(() => {
            Thread.Sleep(TimeSpan.FromSeconds(Math.Max(1, seconds)));
            TimeUp = true;
            MemGen.StopFlag = true;
            Thread.Sleep(TimeSpan.FromSeconds(Math.Max(1, grace)));
            try { File.AppendAllText(file + ".watchdog.txt", "the script did not finish in time and was ended by its watchdog\r\n"); } catch (Exception) { }
            foreach (Process p in Process.GetProcessesByName("bahth")) { try { p.Kill(); } catch (Exception) { } finally { p.Dispose(); } }
            Environment.Exit(124);
        });
        t.IsBackground = true;
        t.Name = "mem-watchdog";
        t.Start();
    }

    // A text file another process may be writing (log.txt).
    public static string ReadShared(string path) {
        using (var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
        using (var r = new StreamReader(fs, Encoding.UTF8))
            return r.ReadToEnd();
    }

    static string Mb(double bytes) { return (bytes / 1048576.0).ToString("0.0", Inv); }
    static string Q(string s) { return "\"" + (s ?? "").Replace("\r", " ").Replace("\n", " ").Replace("\"", "\"\"") + "\""; }

    static string CommandLine(IntPtr h) {
        int len;
        NtQueryInformationProcess(h, 60 /* ProcessCommandLineInformation */, IntPtr.Zero, 0, out len);
        if (len <= 0 || len > (1 << 20)) return "?";
        IntPtr buf = Marshal.AllocHGlobal(len);
        try {
            if (NtQueryInformationProcess(h, 60, buf, len, out len) != 0) return "?";
            int chars = (ushort)Marshal.ReadInt16(buf) / 2;
            IntPtr text = Marshal.ReadIntPtr(buf, IntPtr.Size);
            return text == IntPtr.Zero ? "" : Marshal.PtrToStringUni(text, chars);
        }
        finally { Marshal.FreeHGlobal(buf); }
    }

    static string ParentPid(IntPtr h) {
        int size = 6 * IntPtr.Size, len;
        IntPtr buf = Marshal.AllocHGlobal(size);
        try {
            if (NtQueryInformationProcess(h, 0 /* ProcessBasicInformation */, buf, size, out len) != 0) return "?";
            return Marshal.ReadIntPtr(buf, 5 * IntPtr.Size).ToInt64().ToString(Inv);
        }
        finally { Marshal.FreeHGlobal(buf); }
    }

    // The threads of a process by the name the program gave them
    // ("index-save=2; content-warm=1; (unnamed)=14").
    static string ThreadNames(ProcessThreadCollection threads) {
        var counts = new SortedDictionary<string, int>(StringComparer.Ordinal);
        try {
            foreach (ProcessThread t in threads) {
                string name = "";
                IntPtr h = OpenThread(0x0800 /* THREAD_QUERY_LIMITED_INFORMATION */, false, (uint)t.Id);
                if (h != IntPtr.Zero) {
                    try {
                        IntPtr text;
                        if (GetThreadDescription(h, out text) >= 0 && text != IntPtr.Zero) { name = Marshal.PtrToStringUni(text) ?? ""; LocalFree(text); }
                    }
                    finally { CloseHandle(h); }
                }
                if (name.Length == 0) name = "(unnamed)";
                int c; counts.TryGetValue(name, out c); counts[name] = c + 1;
            }
        }
        catch (Exception e) { return "?" + e.GetType().Name; }
        var sb = new StringBuilder();
        foreach (KeyValuePair<string, int> kv in counts) { if (sb.Length > 0) sb.Append("; "); sb.Append(kv.Key).Append('=').Append(kv.Value); }
        return sb.ToString();
    }

    // Takes one sample now (also called from the script for 1 s steps).
    public static void SampleNow() {
        lock (Gate) {
            try { Sample(); }
            catch (Exception e) { LastError = e.GetType().Name + ": " + e.Message; }
        }
    }

    static void Sample() {
        long n = ++Samples;
        string common = n.ToString(Inv) + "," + DateTime.UtcNow.ToString("yyyy-MM-ddTHH:mm:ss.fffZ", Inv) + "," + Clock.Elapsed.TotalSeconds.ToString("0.0", Inv) + "," + Q(Version) + "," + Q(Scenario) + "," + Q(Phase);

        int wvCount = 0; double wvPrivate = 0, wvWs = 0;
        foreach (Process p in Process.GetProcessesByName("msedgewebview2")) {
            try { wvPrivate += p.PrivateMemorySize64; wvWs += p.WorkingSet64; wvCount++; }
            catch (Exception) { }
            finally { p.Dispose(); }
        }
        var ms = new MemoryStatus(); ms.dwLength = (uint)Marshal.SizeOf(typeof(MemoryStatus));
        double avail = 0, commit = 0;
        if (GlobalMemoryStatusEx(ref ms)) { avail = ms.ullAvailPhys; commit = (double)ms.ullTotalPageFile - ms.ullAvailPageFile; }
        string tail = wvCount.ToString(Inv) + "," + Mb(wvPrivate) + "," + Mb(wvWs) + "," + Interlocked.Read(ref MemGen.Ops).ToString(Inv) + "," + Mb(Interlocked.Read(ref MemGen.Bytes)) + "," + Mb(avail) + "," + Mb(commit);

        var lines = new List<string>();
        var human = new StringBuilder();
        double total = 0;
        foreach (Process p in Process.GetProcessesByName("bahth")) {
            try {
                int id = p.Id;
                double priv = p.PrivateMemorySize64, ws = p.WorkingSet64, peakWs = p.PeakWorkingSet64, peakPriv = p.PeakPagedMemorySize64;
                ProcessThreadCollection all = p.Threads;
                int handles = p.HandleCount, threads = all.Count;
                string names = ThreadNames(all);
                double cpu = p.TotalProcessorTime.TotalSeconds;
                DateTime started = p.StartTime;
                double privWs = -1; uint gdi = 0, user = 0;
                string exe = "?", cmd = "?", parent = "?";
                IntPtr h = OpenProcess(0x0410, false, id);
                if (h == IntPtr.Zero) h = OpenProcess(0x1000, false, id);
                if (h != IntPtr.Zero) {
                    try {
                        var c = new Counters(); uint cb = (uint)Marshal.SizeOf(typeof(Counters)); c.cb = cb;
                        if (GetProcessMemoryInfo(h, ref c, cb)) {
                            // Zero on a Windows that does not fill the newer fields.
                            if (c.PrivateWorkingSetSize.ToUInt64() > 0) privWs = c.PrivateWorkingSetSize.ToUInt64();
                            if (c.PeakPagefileUsage.ToUInt64() > 0) peakPriv = c.PeakPagefileUsage.ToUInt64();
                        }
                        gdi = GetGuiResources(h, 0); user = GetGuiResources(h, 1);
                        var sb = new StringBuilder(1024); int size = sb.Capacity;
                        if (QueryFullProcessImageName(h, 0, sb, ref size)) exe = sb.ToString();
                        string key = id.ToString(Inv) + ":" + started.Ticks.ToString(Inv);
                        string[] known;
                        if (!Known.TryGetValue(key, out known)) {
                            known = new[] { ParentPid(h), CommandLine(h) };
                            Known[key] = known;
                        }
                        parent = known[0]; cmd = known[1];
                    }
                    finally { CloseHandle(h); }
                }
                total += priv;
                lines.Add(id.ToString(Inv) + "," + parent + "," + started.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", Inv) + "," + Mb(priv) + "," + (privWs < 0 ? "" : Mb(privWs)) + "," + Mb(ws) + "," + Mb(peakWs) + "," + Mb(peakPriv) + "," + handles.ToString(Inv) + "," + threads.ToString(Inv) + "," + gdi.ToString(Inv) + "," + user.ToString(Inv) + "," + cpu.ToString("0.0", Inv) + "," + tail + "," + Q(exe) + "," + Q(cmd) + "," + Q(names));
                if (human.Length > 0) human.Append("; ");
                human.Append("pid ").Append(id).Append(" private ").Append(Mb(priv)).Append(" MB");
                if (privWs >= 0) human.Append(", private WS ").Append(Mb(privWs));
                human.Append(", WS ").Append(Mb(ws)).Append(", ").Append(threads).Append(" threads, ").Append(handles).Append(" handles, cpu ").Append(cpu.ToString("0", Inv)).Append(" s");
            }
            catch (Exception) { /* it exited while being read */ }
            finally { p.Dispose(); }
        }
        if (lines.Count == 0) {
            Pending.Append(common).Append(",0,0,,,,,,,,,,,,,").Append(tail).Append(",,,").Append("\r\n");
            human.Append("no bahth process");
        }
        foreach (string l in lines) Pending.Append(common).Append(',').Append(lines.Count.ToString(Inv)).Append(',').Append(l).Append("\r\n");
        if (wvCount > 0) human.Append("; WebView2: ").Append(wvCount).Append(" processes, ").Append(Mb(wvPrivate)).Append(" MB private");
        LastCount = lines.Count;
        LastPrivateMb = total / 1048576.0;
        LastLine = human.ToString();
        // Kept and written with the next sample when the file is busy.
        try { File.AppendAllText(file, Pending.ToString(), new UTF8Encoding(false)); Pending.Length = 0; }
        catch (IOException) { }
    }
}

public static class MemGen {
    // Churn counters: file operations (create, write call, rename, delete) and bytes written.
    public static long Ops, Bytes, Errors;
    public static long OpsRender, OpsUnity, OpsDownload, OpsDocs;
    // Bytes written by the disk load threads (not file churn: not in Ops).
    public static long LoadBytes;
    public static volatile bool StopFlag;
    public static volatile string LastError = "";
    static readonly List<Thread> Workers = new List<Thread>();

    sealed class Rng {
        ulong s;
        public Rng(ulong seed) { s = seed * 0x9E3779B97F4A7C15UL + 0x2545F4914F6CDD1DUL; if (s == 0) s = 1; }
        public ulong Next() { s ^= s << 13; s ^= s >> 7; s ^= s << 17; return s; }
        public int Next(int n) { return (int)(Next() % (ulong)n); }
        public void Fill(byte[] b) {
            int i = 0;
            while (i + 8 <= b.Length) { ulong v = Next(); for (int k = 0; k < 8; k++) b[i++] = (byte)(v >> (8 * k)); }
            while (i < b.Length) b[i++] = (byte)Next();
        }
        public string Hex(int bytes) {
            var sb = new StringBuilder(bytes * 2);
            for (int i = 0; i < bytes; i++) sb.Append(((int)(Next() & 0xFF)).ToString("x2"));
            return sb.ToString();
        }
    }

    // Synthetic words only.
    static readonly string[] En = { "render", "frame", "scene", "mesh", "texture", "shader", "light", "camera", "asset", "bundle", "library", "cache", "import", "export", "build", "editor", "project", "package", "module", "index", "search", "file", "folder", "report", "budget", "invoice", "meeting", "notes", "draft", "final", "version", "update", "release", "server", "client", "error", "warning", "info", "debug", "trace", "start", "stop", "queue", "worker", "thread", "memory", "buffer", "stream", "vector", "matrix", "sample", "pixel", "tile", "layer", "node", "graph", "material", "animation", "rig", "bone", "keyframe", "timeline", "audio", "clip", "prefab", "component", "script", "object", "world", "level", "terrain", "water", "sky", "cloud", "shadow", "reflection", "the", "and", "with", "from" };
    static readonly string[] Ar = { "تقرير", "ميزانية", "مشروع", "ملف", "بحث", "سريع", "بيانات", "اختبار", "مستند", "صورة", "نموذج", "إطار", "مشهد", "خامة", "إضاءة", "تحريك", "تصيير", "مكتبة", "حزمة", "فهرس" };

    static void Words(StringBuilder sb, Rng rng, int count) {
        for (int i = 0; i < count; i++) {
            if (i > 0) sb.Append(' ');
            sb.Append(rng.Next(7) == 0 ? Ar[rng.Next(Ar.Length)] : En[rng.Next(En.Length)]);
        }
    }

    // About `size` bytes of whole lines. kind: 0 text, 1 JSON records, 2 log lines, 3 CSV rows.
    public static byte[] TextBlock(int kind, int size, int seed) {
        var rng = new Rng((ulong)seed + 17);
        var sb = new StringBuilder(size + 512);
        int n = 0;
        while (sb.Length < size) {
            n++;
            if (kind == 1) {
                sb.Append("{\"id\":").Append(n).Append(",\"guid\":\"").Append(rng.Hex(16)).Append("\",\"name\":\"");
                Words(sb, rng, 3);
                sb.Append("\",\"tags\":[\"").Append(En[rng.Next(En.Length)]).Append("\",\"").Append(En[rng.Next(En.Length)]).Append("\"],\"value\":").Append(rng.Next(100000)).Append('.').Append(rng.Next(100)).Append(",\"note\":\"");
                Words(sb, rng, 8);
                sb.Append("\"},\n");
            } else if (kind == 2) {
                sb.Append("2026-01-").Append((1 + n % 28).ToString("D2")).Append('T').Append((n / 3600 % 24).ToString("D2")).Append(':').Append((n / 60 % 60).ToString("D2")).Append(':').Append((n % 60).ToString("D2")).Append("Z ");
                sb.Append(n % 17 == 0 ? "WARN " : "INFO ").Append(En[rng.Next(En.Length)]).Append(": ");
                Words(sb, rng, 9);
                sb.Append(" id=").Append(rng.Hex(6)).Append('\n');
            } else if (kind == 3) {
                sb.Append(n).Append(',').Append(En[rng.Next(En.Length)]).Append(',').Append(Ar[rng.Next(Ar.Length)]).Append(',').Append(rng.Next(1000000)).Append('.').Append(rng.Next(100)).Append(",2026-01-").Append((1 + n % 28).ToString("D2")).Append(',');
                Words(sb, rng, 5);
                sb.Append(',').Append(rng.Hex(8)).Append('\n');
            } else {
                Words(sb, rng, 8 + rng.Next(9));
                sb.Append(n % 9 == 0 ? ".\n\n" : ".\n");
            }
        }
        return Encoding.UTF8.GetBytes(sb.ToString());
    }

    static byte[] Ascii(string s) { return Encoding.ASCII.GetBytes(s); }

    // A big file of real text: 4 MB blocks of lines, each block led by a line of its own.
    public static long BigText(string path, long bytes, int kind, int seed) {
        byte[] block = TextBlock(kind, 4 << 20, seed);
        Directory.CreateDirectory(Path.GetDirectoryName(path));
        using (var fs = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read, 1 << 20, FileOptions.SequentialScan)) {
            if (kind == 1) { byte[] h = Ascii("[\n"); fs.Write(h, 0, h.Length); }
            if (kind == 3) { byte[] h = Ascii("row,word,arabic,amount,date,text,code\n"); fs.Write(h, 0, h.Length); }
            long n = 0;
            while (fs.Position < bytes) {
                n++;
                byte[] mark;
                if (kind == 1) mark = Ascii("{\"block\":" + n + "},\n");
                else if (kind == 2) mark = Ascii("2026-01-01T00:00:00Z INFO block: " + n + "\n");
                else if (kind == 3) mark = Ascii(n + ",block,,0.0,2026-01-01,,\n");
                else mark = Ascii("--- block " + n + " ---\n");
                fs.Write(mark, 0, mark.Length);
                int len = (int)Math.Min(block.Length, Math.Max(1, bytes - fs.Position));
                if (len < block.Length) len = Array.LastIndexOf(block, (byte)'\n', len - 1) + 1;
                if (len <= 0) break;
                fs.Write(block, 0, len);
            }
            if (kind == 1) { byte[] t = Ascii("{\"end\":true}\n]\n"); fs.Write(t, 0, t.Length); }
            return fs.Position;
        }
    }

    // A big binary file: a made-up header, then pseudo-random bytes.
    public static long BigBinary(string path, long bytes, string header, int seed) {
        var rng = new Rng((ulong)seed + 99);
        byte[] block = new byte[4 << 20]; rng.Fill(block);
        Directory.CreateDirectory(Path.GetDirectoryName(path));
        using (var fs = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read, 1 << 20, FileOptions.SequentialScan)) {
            byte[] h = Ascii(header); fs.Write(h, 0, h.Length);
            long n = 0;
            while (fs.Position < bytes) {
                byte[] c = BitConverter.GetBytes(++n);
                Buffer.BlockCopy(c, 0, block, 0, 8);
                fs.Write(block, 0, (int)Math.Min(block.Length, bytes - fs.Position));
            }
            return fs.Position;
        }
    }

    static void WriteSmall(string path, byte[] pool, int off, int len) {
        using (var f = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read, 1)) f.Write(pool, off, len);
    }

    // A deep tree of small files like a Unity project's Library/ and a
    // node_modules folder: `files` files in folders of 250, on all cores,
    // for at most `maxSeconds`. Returns how many were made.
    public static long SmallTree(string root, int files, int maxSeconds) {
        const int perFolder = 250;
        int folders = (files + perFolder - 1) / perFolder;
        byte[] bin = new byte[64 << 10]; new Rng(7).Fill(bin);
        byte[] text = TextBlock(0, 256 << 10, 5);
        DateTime deadline = DateTime.UtcNow.AddSeconds(maxSeconds);
        long made = 0;
        string unity = Path.Combine(root, "UnityProject"), web = Path.Combine(root, "webapp");
        Parallel.For(0, folders, new ParallelOptions { MaxDegreeOfParallelism = Math.Max(2, Environment.ProcessorCount) }, b => {
            if (DateTime.UtcNow > deadline) return;
            var rng = new Rng((ulong)b + 1000);
            int kind = b % 10;
            string dir;
            if (kind < 5) dir = Path.Combine(unity, "Library", "Artifacts", (b % 256).ToString("x2"), (b / 256).ToString("x4"));
            else if (kind == 5) dir = Path.Combine(Path.Combine(unity, "Library", "PackageCache", "com.unity.module" + (b % 40) + "@1." + (b % 7) + ".0"), "Runtime", "Scripts", "Detail" + b);
            else if (kind == 6) dir = Path.Combine(unity, "Library", "ShaderCache", "shader", (b % 16).ToString("x1"), b.ToString("x5"));
            else dir = Path.Combine(Path.Combine(web, "node_modules", "pkg-" + (b % 300), "node_modules"), "dep-" + (b % 90), "lib", "v" + (b % 5), "m" + b);
            try {
                Directory.CreateDirectory(dir);
                for (int i = 0; i < perFolder; i++) {
                    if (Interlocked.Read(ref made) >= files) return;
                    string name; bool isText = false;
                    if (kind < 5) name = rng.Hex(16);
                    else if (kind == 5) { name = (i % 2 == 0) ? "Class" + i + ".cs" : "Class" + (i - 1) + ".cs.meta"; isText = true; }
                    else if (kind == 6) name = rng.Hex(8) + ".bin";
                    else {
                        int m = i % 6;
                        name = m == 0 ? "index" + i + ".js" : m == 1 ? "util" + i + ".js" : m == 2 ? "types" + i + ".d.ts" : m == 3 ? "package" + i + ".json" : m == 4 ? "README" + i + ".md" : "chunk" + i + ".map";
                        isText = true;
                    }
                    int len = isText ? 200 + rng.Next(1800) : 64 + rng.Next(2000);
                    byte[] pool = isText ? text : bin;
                    WriteSmall(Path.Combine(dir, name), pool, rng.Next(pool.Length - len), len);
                    Interlocked.Increment(ref made);
                }
            }
            catch (Exception e) { Interlocked.Increment(ref Errors); LastError = e.GetType().Name + ": " + e.Message; }
        });
        return Interlocked.Read(ref made);
    }

    // ---- zip, written by hand (stored and deflated members; no zip64: keep it under 4 GB)
    static uint[] crcTable;
    static uint Crc(uint crc, byte[] b, int off, int len) {
        if (crcTable == null) {
            var t = new uint[256];
            for (uint i = 0; i < 256; i++) { uint c = i; for (int k = 0; k < 8; k++) c = (c & 1) != 0 ? 0xEDB88320u ^ (c >> 1) : c >> 1; t[i] = c; }
            crcTable = t;
        }
        for (int i = off; i < off + len; i++) crc = crcTable[(crc ^ b[i]) & 0xFF] ^ (crc >> 8);
        return crc;
    }

    sealed class ZipMember { public byte[] Name; public uint Crc, CSize, USize, Offset; public ushort Method; }

    // One member of `usize` bytes taken from `block` (again and again), starting at `off`.
    static void ZipAdd(FileStream fs, BinaryWriter w, List<ZipMember> members, string name, bool deflate, byte[] block, int off, long usize) {
        var m = new ZipMember { Name = Encoding.UTF8.GetBytes(name), Method = (ushort)(deflate ? 8 : 0), Offset = (uint)fs.Position };
        w.Write(0x04034b50u); w.Write((ushort)20); w.Write((ushort)0x0800); w.Write(m.Method); w.Write((ushort)0); w.Write((ushort)0x5821);
        w.Write(0u); w.Write(0u); w.Write(0u); w.Write((ushort)m.Name.Length); w.Write((ushort)0); w.Write(m.Name); w.Flush();
        long dataStart = fs.Position; uint crc = 0xFFFFFFFFu; long left = usize;
        Stream target = deflate ? (Stream)new DeflateStream(fs, CompressionLevel.Fastest, true) : fs;
        while (left > 0) {
            int len = (int)Math.Min(block.Length - off, left);
            target.Write(block, off, len);
            crc = Crc(crc, block, off, len);
            left -= len; off = 0;
        }
        if (deflate) target.Dispose();
        fs.Flush();
        long end = fs.Position;
        m.Crc = ~crc; m.CSize = (uint)(end - dataStart); m.USize = (uint)usize;
        fs.Position = m.Offset + 14; w.Write(m.Crc); w.Write(m.CSize); w.Write(m.USize); w.Flush();
        fs.Position = end;
        members.Add(m);
    }

    // A zip with `small` deflated text members of `smallBytes` each, one
    // deflated member of `deflatedHuge` bytes of text, and one stored
    // member of `storedHuge` bytes. Returns the size of the zip.
    public static long Zip(string path, int small, int smallBytes, long deflatedHuge, long storedHuge, int seed) {
        byte[] block = TextBlock(0, 4 << 20, seed);
        var members = new List<ZipMember>();
        Directory.CreateDirectory(Path.GetDirectoryName(path));
        using (var fs = new FileStream(path, FileMode.Create, FileAccess.ReadWrite, FileShare.Read, 1 << 20)) {
            var w = new BinaryWriter(fs);
            for (int i = 0; i < small; i++) {
                int off = (int)(((long)i * 7919) % (block.Length - smallBytes));
                ZipAdd(fs, w, members, "docs/part-" + (i / 200).ToString("D3") + "/note-" + i.ToString("D5") + ((i % 3 == 0) ? ".md" : ".txt"), true, block, off, smallBytes);
            }
            if (deflatedHuge > 0) ZipAdd(fs, w, members, "huge/editor-dump.log", true, block, 0, deflatedHuge);
            if (storedHuge > 0) ZipAdd(fs, w, members, "huge/scene-export.txt", false, block, 0, storedHuge);
            long cdStart = fs.Position;
            foreach (ZipMember m in members) {
                w.Write(0x02014b50u); w.Write((ushort)20); w.Write((ushort)20); w.Write((ushort)0x0800); w.Write(m.Method); w.Write((ushort)0); w.Write((ushort)0x5821);
                w.Write(m.Crc); w.Write(m.CSize); w.Write(m.USize); w.Write((ushort)m.Name.Length); w.Write((ushort)0); w.Write((ushort)0); w.Write((ushort)0); w.Write((ushort)0); w.Write(0u); w.Write(m.Offset); w.Write(m.Name);
            }
            w.Flush();
            long cdSize = fs.Position - cdStart;
            w.Write(0x06054b50u); w.Write((ushort)0); w.Write((ushort)0); w.Write((ushort)members.Count); w.Write((ushort)members.Count); w.Write((uint)cdSize); w.Write((uint)cdStart); w.Write((ushort)0);
            w.Flush();
            return fs.Position;
        }
    }

    // ---- tar.gz
    static void Put(byte[] h, int at, string s) { byte[] b = Ascii(s); Buffer.BlockCopy(b, 0, h, at, b.Length); }
    static void TarAdd(Stream gz, string name, byte[] block, int off, long size) {
        byte[] h = new byte[512];
        Put(h, 0, name); Put(h, 100, "0000644"); Put(h, 108, "0000000"); Put(h, 116, "0000000");
        Put(h, 124, Convert.ToString(size, 8).PadLeft(11, '0')); Put(h, 136, "14540000000");
        for (int i = 148; i < 156; i++) h[i] = 32;
        h[156] = (byte)'0'; Put(h, 257, "ustar"); h[263] = (byte)'0'; h[264] = (byte)'0';
        int sum = 0; foreach (byte b in h) sum += b;
        Put(h, 148, Convert.ToString(sum, 8).PadLeft(6, '0')); h[154] = 0; h[155] = 32;
        gz.Write(h, 0, 512);
        long left = size;
        while (left > 0) { int len = (int)Math.Min(block.Length - off, left); gz.Write(block, off, len); left -= len; off = 0; }
        int pad = (int)((512 - size % 512) % 512);
        if (pad > 0) gz.Write(new byte[pad], 0, pad);
    }

    // A tar.gz with `entries` text members of `entryBytes` each and one of `hugeBytes`.
    public static long TarGz(string path, int entries, int entryBytes, long hugeBytes, int seed) {
        byte[] block = TextBlock(2, 4 << 20, seed);
        Directory.CreateDirectory(Path.GetDirectoryName(path));
        using (var fs = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read, 1 << 20)) {
            using (var gz = new GZipStream(fs, CompressionLevel.Fastest, true)) {
                for (int i = 0; i < entries; i++) {
                    int off = (int)(((long)i * 104729) % (block.Length - entryBytes));
                    TarAdd(gz, "logs/day-" + (i / 100).ToString("D3") + "/run-" + i.ToString("D5") + ".log", block, off, entryBytes);
                }
                if (hugeBytes > 0) TarAdd(gz, "logs/huge-session.log", block, 0, hugeBytes);
                gz.Write(new byte[1024], 0, 1024);
            }
            fs.Flush();
            return fs.Position;
        }
    }

    // ---- churn
    static void Fail(Exception e) {
        Interlocked.Increment(ref Errors);
        LastError = e.GetType().Name + ": " + e.Message;
        Thread.Sleep(20);
    }
    static void Count(ref long kind, long ops, long bytes) {
        Interlocked.Add(ref Ops, ops); Interlocked.Add(ref kind, ops);
        if (bytes > 0) Interlocked.Add(ref Bytes, bytes);
    }
    // Sleeps until `written` bytes are due at `mbps`.
    static void Pace(Stopwatch sw, long written, double mbps) {
        while (!StopFlag && written / 1048576.0 > mbps * sw.Elapsed.TotalSeconds) Thread.Sleep(5);
    }
    static void TryDelete(string path, ref long kind) {
        try { if (File.Exists(path)) { File.Delete(path); Count(ref kind, 1, 0); } }
        catch (Exception e) { Fail(e); }
    }

    // A renderer: numbered frames of 2-20 MB (.png, every third .exr) with
    // .txt and .json sidecars; the last six stay, an older one is written
    // over now and then.
    static void RenderLoop(string dir, int id, double mbps) {
        Directory.CreateDirectory(dir);
        var rng = new Rng((ulong)(300 + id));
        byte[] buf = new byte[1 << 20]; rng.Fill(buf);
        byte[] text = TextBlock(0, 64 << 10, 40 + id), json = TextBlock(1, 64 << 10, 50 + id);
        var sw = Stopwatch.StartNew(); long written = 0; int frame = 0;
        var live = new Queue<int>();
        while (!StopFlag) {
            try {
                frame++;
                int target = frame;
                // Every seventh turn renders an earlier frame again instead.
                if (frame % 7 == 0 && live.Count > 2) target = frame - 2;
                string stem = Path.Combine(dir, "frame_" + target.ToString("D6"));
                string image = stem + (target % 3 == 0 ? ".exr" : ".png");
                long size = (2 + rng.Next(19)) * (1L << 20);
                using (var fs = new FileStream(image, FileMode.Create, FileAccess.Write, FileShare.Read, 4096)) {
                    Count(ref OpsRender, 1, 0);
                    for (long done = 0; done < size && !StopFlag; done += buf.Length) {
                        buf[0] = (byte)frame; buf[1] = (byte)(done >> 20);
                        fs.Write(buf, 0, buf.Length); fs.Flush();
                        written += buf.Length; Count(ref OpsRender, 1, buf.Length);
                        Pace(sw, written, mbps);
                    }
                }
                WriteSmall(stem + ".txt", text, rng.Next(text.Length - 4096), 1024 + rng.Next(3000));
                WriteSmall(stem + ".json", json, 0, 2048 + rng.Next(8000));
                Count(ref OpsRender, 4, 8192);
                if (target == frame) live.Enqueue(frame);
                while (live.Count > 6) {
                    string old = Path.Combine(dir, "frame_" + live.Dequeue().ToString("D6"));
                    TryDelete(old + ".png", ref OpsRender); TryDelete(old + ".exr", ref OpsRender); TryDelete(old + ".txt", ref OpsRender); TryDelete(old + ".json", ref OpsRender);
                }
            }
            catch (Exception e) { Fail(e); }
        }
    }

    // Unity's Library: small files written as .tmp and renamed into deep
    // hashed folders, an .info beside some, the oldest deleted; and
    // short-lived Temp folders of a few dozen files.
    static void UnityLoop(string root, int id, int perSecond, int keep) {
        string lib = Path.Combine(root, "Library"), temp = Path.Combine(root, "Temp");
        var rng = new Rng((ulong)(500 + id));
        byte[] buf = new byte[16 << 10]; rng.Fill(buf);
        byte[] text = TextBlock(1, 32 << 10, 60 + id);
        var sw = Stopwatch.StartNew(); long n = 0; string lastTemp = null;
        var live = new Queue<string>();
        while (!StopFlag) {
            try {
                n++;
                string hash = rng.Hex(16);
                string dir = Path.Combine(lib, "Artifacts", hash.Substring(0, 2), hash.Substring(2, 2));
                Directory.CreateDirectory(dir);
                string tmp = Path.Combine(dir, hash + ".tmp"), fin = Path.Combine(dir, hash);
                int len = 200 + rng.Next(3800);
                WriteSmall(tmp, buf, rng.Next(buf.Length - len), len);
                File.Move(tmp, fin);
                Count(ref OpsUnity, 3, len);
                live.Enqueue(fin);
                if (n % 4 == 0) {
                    WriteSmall(fin + ".info", text, rng.Next(text.Length - 2048), 300 + rng.Next(1500));
                    Count(ref OpsUnity, 2, 1000);
                    live.Enqueue(fin + ".info");
                }
                while (live.Count > keep) TryDelete(live.Dequeue(), ref OpsUnity);
                if (n % 300 == 0) {
                    string t = Path.Combine(temp, "ILPP-" + id + "-" + n);
                    Directory.CreateDirectory(t);
                    for (int i = 0; i < 25; i++) WriteSmall(Path.Combine(t, "Assembly" + i + (i % 3 == 0 ? ".dll" : i % 3 == 1 ? ".pdb" : ".json")), buf, 0, 4096 + rng.Next(8000));
                    Count(ref OpsUnity, 51, 25 * 8000);
                    if (lastTemp != null) { Directory.Delete(lastTemp, true); Count(ref OpsUnity, 26, 0); }
                    lastTemp = t;
                }
                while (!StopFlag && n > perSecond * sw.Elapsed.TotalSeconds) Thread.Sleep(2);
            }
            catch (Exception e) { Fail(e); }
        }
    }

    // A downloader: three text files (.log .txt .json) that grow by 2 MB
    // appends up to `cap` bytes and start again, and 4 MB chunk files
    // written as .part, renamed when complete, the oldest deleted.
    static void DownloadLoop(string dir, long cap, double mbps) {
        string chunks = Path.Combine(dir, "chunks");
        Directory.CreateDirectory(chunks);
        string[] exts = { ".log", ".txt", ".json" };
        byte[][] blocks = { TextBlock(2, 2 << 20, 70), TextBlock(0, 2 << 20, 71), TextBlock(1, 2 << 20, 72) };
        string[] paths = new string[3]; long[] sizes = new long[3]; int[] gen = new int[3];
        var rng = new Rng(700);
        byte[] bin = new byte[1 << 20]; rng.Fill(bin);
        var sw = Stopwatch.StartNew(); long written = 0; int c = 0;
        var live = new Queue<string>();
        while (!StopFlag) {
            try {
                for (int k = 0; k < 3 && !StopFlag; k++) {
                    if (paths[k] == null) { gen[k]++; paths[k] = Path.Combine(dir, "download-" + k + "-" + gen[k].ToString("D3") + exts[k]); sizes[k] = 0; }
                    using (var fs = new FileStream(paths[k], FileMode.Append, FileAccess.Write, FileShare.Read, 4096)) fs.Write(blocks[k], 0, blocks[k].Length);
                    sizes[k] += blocks[k].Length; written += blocks[k].Length;
                    Count(ref OpsDownload, 1, blocks[k].Length);
                    if (sizes[k] >= cap) { TryDelete(paths[k], ref OpsDownload); paths[k] = null; }
                    Pace(sw, written, mbps);
                }
                c++;
                string part = Path.Combine(chunks, "chunk_" + c.ToString("D6") + ".part"), done = Path.Combine(chunks, "chunk_" + c.ToString("D6") + ".bin");
                using (var fs = new FileStream(part, FileMode.Create, FileAccess.Write, FileShare.Read, 4096)) {
                    for (int i = 0; i < 4; i++) { bin[0] = (byte)c; fs.Write(bin, 0, bin.Length); fs.Flush(); }
                }
                if (File.Exists(done)) File.Delete(done);
                File.Move(part, done);
                written += 4 * bin.Length;
                Count(ref OpsDownload, 6, 4 * bin.Length);
                live.Enqueue(done);
                while (live.Count > 24) TryDelete(live.Dequeue(), ref OpsDownload);
                Pace(sw, written, mbps);
            }
            catch (Exception e) { Fail(e); }
        }
    }

    // Documents saved again and again: in place, or as a temporary file
    // that replaces the original (the way office programs save).
    static void DocsLoop(string dir, int files, int perSecond) {
        Directory.CreateDirectory(dir);
        string[] exts = { ".txt", ".md", ".json", ".csv", ".log" };
        int[] kinds = { 0, 0, 1, 3, 2 };
        byte[][] blocks = { TextBlock(0, 1 << 20, 80), TextBlock(1, 1 << 20, 81), TextBlock(2, 1 << 20, 82), TextBlock(3, 1 << 20, 83) };
        var rng = new Rng(900);
        var sw = Stopwatch.StartNew(); long n = 0;
        while (!StopFlag) {
            try {
                n++;
                int i = rng.Next(files);
                string path = Path.Combine(dir, "notes_" + i.ToString("D3") + exts[i % exts.Length]);
                byte[] block = blocks[kinds[i % exts.Length]];
                int len = (i % 10 == 0) ? 200000 + rng.Next(800000) : 2000 + rng.Next(60000);
                int off = rng.Next(block.Length - len);
                if (n % 3 == 0 && File.Exists(path)) {
                    string tmp = Path.Combine(dir, "~$notes_" + i.ToString("D3") + ".tmp");
                    WriteSmall(tmp, block, off, len);
                    File.Delete(path);
                    File.Move(tmp, path);
                    Count(ref OpsDocs, 4, len);
                } else {
                    WriteSmall(path, block, off, len);
                    Count(ref OpsDocs, 2, len);
                }
                while (!StopFlag && n > perSecond * sw.Elapsed.TotalSeconds) Thread.Sleep(5);
            }
            catch (Exception e) { Fail(e); }
        }
    }

    // (Above normal: the churn keeps its pace while the CPU burners run.)
    static void Run(string name, ThreadStart body, ThreadPriority priority = ThreadPriority.AboveNormal) {
        var t = new Thread(body);
        t.IsBackground = true; t.Name = name; t.Priority = priority;
        Workers.Add(t);
        t.Start();
    }

    // Keeps `threads` cores busy with normal-priority work, as renderers
    // and compilers do. Stops with StopChurn.
    public static void StartCpuBurners(int threads) {
        StopFlag = false;
        for (int i = 0; i < threads; i++) {
            Run("cpu-burner-" + i, () => {
                ulong x = 1;
                while (!StopFlag) { for (int k = 0; k < 2000000; k++) x = x * 6364136223846793005UL + 1442695040888963407UL; }
                GC.KeepAlive(x);
            }, ThreadPriority.Normal);
        }
    }

    // A burst of changes: `files` small files in folders of 500, written on
    // twice as many threads as cores, as fast as the disk takes them; with
    // `delete`, all of them are deleted again at once. Returns the number
    // of file operations.
    public static long Burst(string root, int files, bool delete) {
        byte[] bin = new byte[4096]; new Rng(11).Fill(bin);
        long made = 0;
        int folders = (files + 499) / 500;
        Parallel.For(0, folders, new ParallelOptions { MaxDegreeOfParallelism = Math.Max(4, Environment.ProcessorCount * 2) }, b => {
            try {
                string dir = Path.Combine(root, "b" + (b % 64).ToString("D2"), "f" + b.ToString("D5"));
                Directory.CreateDirectory(dir);
                for (int i = 0; i < 500 && Interlocked.Read(ref made) < files; i++) {
                    WriteSmall(Path.Combine(dir, "burst-" + b + "-" + i + ".tmp"), bin, 0, 64 + (i % 200));
                    Interlocked.Increment(ref made);
                }
            }
            catch (Exception e) { Fail(e); }
        });
        long ops = Interlocked.Read(ref made) * 2;
        if (delete) {
            try { Directory.Delete(root, true); ops += Interlocked.Read(ref made); }
            catch (Exception e) { Fail(e); }
        }
        Interlocked.Add(ref Ops, ops);
        return ops;
    }

    // Starts the churn under `root`: `renderers` render threads at
    // `renderMbps` each, `unity` Library threads at `unityPerSecond` files a
    // second each, one downloader at `downloadMbps` whose files grow to
    // `growCap` bytes, and one thread saving `docsPerSecond` documents a second.
    public static void StartChurn(string root, long growCap, int renderers, double renderMbps, int unity, int unityPerSecond, double downloadMbps, int docsPerSecond) {
        StopFlag = false;
        Ops = 0; Bytes = 0; Errors = 0; OpsRender = 0; OpsUnity = 0; OpsDownload = 0; OpsDocs = 0; LastError = "";
        Workers.Clear();
        Directory.CreateDirectory(root);
        for (int i = 0; i < renderers; i++) { int id = i; Run("churn-render-" + id, () => RenderLoop(Path.Combine(root, "renders", "shot_" + id.ToString("D2")), id, renderMbps)); }
        for (int i = 0; i < unity; i++) { int id = i; Run("churn-unity-" + id, () => UnityLoop(Path.Combine(root, "UnityProject"), id, unityPerSecond, 8000)); }
        Run("churn-download", () => DownloadLoop(Path.Combine(root, "Downloads"), growCap, downloadMbps));
        Run("churn-docs", () => DocsLoop(Path.Combine(root, "docs"), 300, docsPerSecond));
    }

    // Keeps a disk busy: each thread writes 4 MB blocks one after another,
    // unbuffered (write-through), into its own file of at most `cap` bytes,
    // starting again at the top when it is full. Stops with StopChurn.
    public static void StartDiskLoad(string dir, int threads, long cap) {
        StopFlag = false;
        LoadBytes = 0;
        Directory.CreateDirectory(dir);
        for (int i = 0; i < threads; i++) {
            int id = i;
            Run("disk-load-" + id, () => {
                var rng = new Rng((ulong)(40 + id));
                byte[] block = new byte[4 << 20]; rng.Fill(block);
                while (!StopFlag) {
                    try {
                        using (var fs = new FileStream(Path.Combine(dir, "load-" + id + ".bin"), FileMode.Create, FileAccess.Write, FileShare.Read, 4096, FileOptions.WriteThrough)) {
                            while (!StopFlag) {
                                if (fs.Position + block.Length > cap) fs.Position = 0;
                                fs.Write(block, 0, block.Length);
                                fs.Flush();
                                Interlocked.Add(ref LoadBytes, block.Length);
                            }
                        }
                    }
                    catch (Exception e) { Fail(e); Thread.Sleep(500); }
                }
            });
        }
    }

    public static void StopChurn() {
        StopFlag = true;
        foreach (Thread t in Workers) t.Join(60000);
        Workers.Clear();
    }
}
'@
# Real pixels, whatever the display scale.
[void][MemUi]::SetProcessDPIAware()

# ---------------------------------------------------------------- set-up
$VK = @{ CTRL = 0x11; ALT = 0x12; ENTER = 0x0D; ESC = 0x1B; A = 0x41; F = 0x46; BACK = 0x08 }
$BM_CLICK = 0x00F5; $BM_GETCHECK = 0x00F0; $WM_CLOSE = 0x0010; $WM_CANCELMODE = 0x001F; $WM_MENUCOMMAND = 0x0126

New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$logsDir = Join-Path $Out 'logs'
$shotsDir = Join-Path $Out 'shots'
New-Item -ItemType Directory -Force $logsDir, $shotsDir | Out-Null
$samplesFile = Join-Path $Out 'samples.csv'
$diagFile = Join-Path $Out 'diagnostics.txt'

$data = Join-Path $env:LOCALAPPDATA 'BahthSari'
$settingsFile = Join-Path $data 'settings.txt'
$logFile = Join-Path $data 'log.txt'
$crashFile = Join-Path $data 'crash.txt'
$indexFile = Join-Path $data 'index.bin'
# The program's own permanent place (src/relocate.rs): a copy there is never
# asked to move, and it is where a real installed copy updates itself.
$instDir = Join-Path $env:LOCALAPPDATA 'Programs\InstaFind'
$instExe = Join-Path $instDir 'bahth.exe'
# A second copy, for starting the other version while one is running.
$sideDir = Join-Path $env:LOCALAPPDATA 'Programs\InstaFind-other'
$sideExe = Join-Path $sideDir 'bahth.exe'

$rows = New-Object System.Collections.Generic.List[object]
$churnRows = New-Object System.Collections.Generic.List[object]
$notes = New-Object System.Collections.Generic.List[string]
$warnings = New-Object System.Collections.Generic.List[string]
$problems = New-Object System.Collections.Generic.List[string]
$crashes = New-Object System.Collections.Generic.List[string]
$details = New-Object Text.StringBuilder
$criteria = New-Object System.Collections.Generic.List[object]
$script:criteriaFailed = 0
$script:shot = 0
$script:crashN = 0
$script:logSeen = ''
$script:logAll = ''
$script:logCopy = Join-Path $logsDir 'unnamed.txt'
$script:app = $null
$script:optionsMenu = $null
$script:dialogsDismissed = 0
$script:tree = $null
$script:bigFiles = $null
# Saves of index.bin seen from outside (Poll-IndexSave).
$script:save = @{ TmpSince = $null; LastWrite = $null; Count = 0; Longest = 0.0; Lines = (New-Object System.Collections.Generic.List[string]) }

function Note([string]$Text) { $t = One-Line $Text; $notes.Add($t); Say "NOTE: $t" }
function Warn([string]$Text) { $t = One-Line $Text; $warnings.Add($t); Write-Host "::warning::$t" }
function Problem([string]$Text) { $t = One-Line $Text; $problems.Add($t); Write-Host "::error::mem-test harness: $t" }
function Crash([string]$Text) { $t = One-Line $Text; $crashes.Add($t); Write-Host "::error::bahth crashed: $t" }
function Detail([string]$Text) { [void]$details.AppendLine($Text) }
# One of the review council's pass criteria. They are set for the branch
# build: a FAIL there is a warning (and fails the run with -FailOnCriteria);
# the released exes are measured the same way and only reported.
function Add-Criterion([string]$Scenario, [string]$Version, [string]$Criterion, [bool]$Pass, [string]$Value) {
    $result = if ($Pass) { 'PASS' } else { 'FAIL' }
    $criteria.Add([pscustomobject]@{ Scenario = $Scenario; Version = $Version; Criterion = $Criterion; Result = $result; Value = (One-Line $Value); Leg = $Leg })
    Say "CRITERION [$Version] $Scenario / ${Criterion}: $result ($(One-Line $Value))"
    if (-not $Pass -and $Version -eq 'branch') {
        $script:criteriaFailed++
        Warn "the branch build FAILS a criterion: $Scenario / ${Criterion}: $(One-Line $Value)"
    }
}

# How long to wait for `Want` seconds of something, after -TimeScale (never
# under `Min`: what an event needs) and what is left of -BudgetMinutes
# (`Reserve` seconds are kept for the report).
function Plan-Seconds([double]$Want, [string]$What, [int]$Reserve = 240, [double]$Min = 1.0) {
    $s = [Math]::Max($Min, $Want * $TimeScale)
    if ($BudgetMinutes -gt 0) {
        $left = ($script:t0.AddMinutes($BudgetMinutes) - (Get-Date)).TotalSeconds - $Reserve
        if ($left -lt $s) {
            $new = [Math]::Max(5.0, $left)
            Note ("time budget ({0} min): '{1}' shortened from {2:N0} s to {3:N0} s" -f $BudgetMinutes, $What, $s, $new)
            $s = $new
        }
    }
    return [int][Math]::Ceiling($s)
}

# The time budget is used up: loops stop, scenarios that are left are skipped.
function Time-Up {
    if ($BudgetMinutes -le 0) { return $false }
    return ([MemSampler]::TimeUp -or (Get-Date) -gt $script:t0.AddMinutes($BudgetMinutes))
}
# Seconds of the budget that are left (a day when there is no budget).
function Time-Left {
    if ($BudgetMinutes -le 0) { return 86400.0 }
    return ($script:t0.AddMinutes($BudgetMinutes) - (Get-Date)).TotalSeconds
}

# Deletes a folder tree with `rd` (far quicker than Remove-Item on half a
# million files), for at most `Seconds`. True when it is gone.
function Remove-Tree([string]$Path, [int]$Seconds = 150) {
    if (-not (Test-Path -LiteralPath $Path)) { return $true }
    try {
        $p = Start-Process -FilePath $env:ComSpec -ArgumentList "/c rd /s /q `"$Path`"" -WindowStyle Hidden -PassThru
        if (-not $p.WaitForExit($Seconds * 1000)) { try { $p.Kill() } catch { } }
    } catch { Say "removing $Path failed: $_" }
    return (-not (Test-Path -LiteralPath $Path))
}

function Set-Phase([string]$Version, [string]$Scenario, [string]$Phase) {
    [MemSampler]::Version = $Version; [MemSampler]::Scenario = $Scenario; [MemSampler]::Phase = $Phase
    Say ">>> $Version / $Scenario / $Phase"
}

function Shot([string]$Name) {
    $script:shot++
    $file = Join-Path $shotsDir ('{0:D2}-{1}.png' -f $script:shot, ($Name -replace '[^A-Za-z0-9.-]+', '-'))
    try {
        $b = [Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
        $bmp.Save($file, [Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    } catch { Say "screenshot $Name failed: $_" }
}

function Wait-Until([scriptblock]$Cond, [double]$Seconds = 10) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) {
        $v = & $Cond
        if ($v) { return $v }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

# ---------------------------------------------------------------- windows
function Parse-Win([string]$Line) {
    $p = $Line.Split('|', 8)
    [pscustomobject]@{ Handle = [IntPtr][long]$p[0]; Class = $p[1]; Visible = $p[2] -eq 'True'; X = [int]$p[3]; Y = [int]$p[4]; W = [int]$p[5]; H = [int]$p[6]; Text = $p[7] }
}
function Get-Windows([int]$ProcId) { foreach ($l in [MemUi]::Windows($ProcId)) { Parse-Win $l } }
function Get-Children($Hwnd) { foreach ($l in [MemUi]::Children($Hwnd)) { Parse-Win $l } }
function Bahth-Ids { @(Get-Process bahth -ErrorAction SilentlyContinue | Sort-Object Id | ForEach-Object { $_.Id }) }
function Bahth-Windows { foreach ($id in (Bahth-Ids)) { Get-Windows $id } }
# The program's main windows (nwg's class), visible and big: the classic
# window, or the new look's.
function Main-Window {
    Bahth-Windows | Where-Object { $_.Visible -and $_.Class -eq 'NativeWindowsGuiWindow' -and $_.W -gt 300 -and $_.H -gt 200 } |
        Sort-Object { $_.W * $_.H } -Descending | Select-Object -First 1
}
# The classic window holds the result list.
function Classic-Window {
    Bahth-Windows | Where-Object { $_.Visible -and $_.Class -eq 'NativeWindowsGuiWindow' -and $_.W -gt 300 -and $_.H -gt 200 } |
        Where-Object { [bool](Get-Children $_.Handle | Where-Object Class -eq 'SysListView32') } | Select-Object -First 1
}
function Popup-Window { Bahth-Windows | Where-Object { $_.Visible -and $_.Class -eq 'InstaFindQuickSearch' } | Select-Object -First 1 }
function Open-Menu { Get-Windows 0 | Where-Object { $_.Class -eq '#32768' } | Select-Object -First 1 }
function Focus-Window($Hwnd) {
    # Windows only hands the foreground to the process that last had input;
    # a tap of Alt counts as that. The tap can leave the window in menu
    # mode, where a popup menu closes at once: leave that mode again.
    [MemUi]::Chord($VK.ALT)
    [void][MemUi]::SetForegroundWindow($Hwnd)
    Start-Sleep -Milliseconds 300
    [void][MemUi]::PostMessage($Hwnd, $WM_CANCELMODE, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
}
function Click-Center($W) { [MemUi]::Click([int]($W.X + $W.W / 2), [int]($W.Y + $W.H / 2), $false) }
# The classic window's controls, found by class (and the buttons by text).
function Find-Controls($Main) {
    $kids = @(Get-Children $Main.Handle | Where-Object Visible)
    $buttons = @($kids | Where-Object Class -eq 'Button')
    [pscustomobject]@{
        Main    = $Main
        Query   = $kids | Where-Object Class -eq 'Edit' | Sort-Object Y | Select-Object -First 1
        List    = $kids | Where-Object Class -eq 'SysListView32' | Select-Object -First 1
        Content = $buttons | Where-Object { $_.Text -match 'inside documents|داخل المستندات' } | Select-Object -First 1
        Options = $buttons | Where-Object { $_.Text -match '^Options|^خيارات' } | Select-Object -First 1
    }
}
# The status bar of the classic window, shown or hidden in the tray: the
# program's own words about indexing.
function Status-Line {
    try {
        foreach ($w in @(Bahth-Windows | Where-Object { $_.Class -eq 'NativeWindowsGuiWindow' })) {
            $bar = Get-Children $w.Handle | Where-Object Class -eq 'msctls_statusbar32' | Select-Object -First 1
            if ($bar) {
                $parts = @(0, 1, 2 | ForEach-Object { [MemUi]::StatusText($bar.Handle, $_) }) | Where-Object { $_ }
                return ($parts -join ' | ')
            }
        }
    } catch { return "(status bar: $_)" }
    return ''
}
# Message boxes of the program: title and text.
function Find-Dialogs {
    foreach ($w in @(Bahth-Windows | Where-Object { $_.Visible -and $_.Class -eq '#32770' })) {
        $body = (Get-Children $w.Handle | Where-Object { $_.Class -eq 'Static' -and $_.Text } | ForEach-Object { $_.Text }) -join ' '
        [pscustomobject]@{ Handle = $w.Handle; Title = $w.Text; Body = $body }
    }
}
# Presses a button of a message box by its id (6 Yes, 7 No, 1 OK, 2 Cancel);
# with no such button, closes the box.
function Answer-Dialog($Dlg, [int[]]$Ids) {
    foreach ($id in $Ids) {
        $b = [MemUi]::GetDlgItem($Dlg.Handle, $id)
        if ($b -ne [IntPtr]::Zero) { [void][MemUi]::PostMessage($b, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero); return $id }
    }
    [void][MemUi]::PostMessage($Dlg.Handle, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    return 0
}
# A message box nobody expected would block the program: say No to it.
function Dismiss-Unexpected {
    foreach ($d in @(Find-Dialogs)) {
        $script:dialogsDismissed++
        if ($script:dialogsDismissed -le 10) {
            Note "unexpected message box '$($d.Title)': $($d.Body)"
            Shot 'unexpected-dialog'
        }
        [void](Answer-Dialog $d @(7, 2, 1))
        Start-Sleep -Milliseconds 500
    }
}

function Process-List {
    try {
        return @(Get-CimInstance Win32_Process -Filter "Name LIKE 'bahth%'" -ErrorAction Stop | Sort-Object CreationDate | ForEach-Object {
            "pid {0}, parent {1}, started {2:HH:mm:ss}, working set {3:N0} MB, {4}" -f $_.ProcessId, $_.ParentProcessId, $_.CreationDate, ($_.WorkingSetSize / 1MB), $_.CommandLine
        })
    } catch {
        $why = $_.Exception.Message
        return @(Get-Process bahth -ErrorAction SilentlyContinue | ForEach-Object { "pid $($_.Id), working set $([int]($_.WorkingSet64 / 1MB)) MB (no command line: $why)" })
    }
}

function Dump-State([string]$When) {
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("==== $When ($(Get-Date -Format s))")
    try {
        foreach ($l in @(Process-List)) { $lines.Add("process: $l") }
        foreach ($id in (Bahth-Ids)) {
            foreach ($w in @(Get-Windows $id)) {
                $lines.Add("window of ${id}: $($w.Handle) $($w.Class) visible=$($w.Visible) $($w.X),$($w.Y) $($w.W)x$($w.H) '$($w.Text)'")
                if ($w.Visible) { foreach ($k in @(Get-Children $w.Handle | Select-Object -First 40)) { $lines.Add("    child $($k.Class) visible=$($k.Visible) $($k.W)x$($k.H) '$($k.Text)'") } }
            }
        }
        $lines.Add('-- settings.txt:')
        if (Test-Path -LiteralPath $settingsFile) { $lines.Add([MemSampler]::ReadShared($settingsFile)) }
        $lines.Add('-- log.txt (last 60 lines):')
        foreach ($l in @($script:logAll -split "`r?`n" | Where-Object { $_ } | Select-Object -Last 60)) { $lines.Add($l) }
        $lines.Add("-- sampler: $([MemSampler]::LastLine); last error: $([MemSampler]::LastError)")
    } catch { $lines.Add("(dump failed: $_)") }
    $text = ($lines -join "`r`n") + "`r`n"
    [IO.File]::AppendAllText($diagFile, $text, $utf8)
    Write-Host $text
}

# ---------------------------------------------------------------- the program's files
# Copies what log.txt gained since the last look to this run's own copy
# (the program deletes log.txt when it passes 64 KB).
function Sync-Log {
    if (-not (Test-Path -LiteralPath $logFile)) { return }
    $text = $null
    try { $text = [MemSampler]::ReadShared($logFile) } catch { return }
    if ($null -eq $text) { return }
    $seen = $script:logSeen
    $new = if ($text.Length -ge $seen.Length -and $text.StartsWith($seen, [StringComparison]::Ordinal)) { $text.Substring($seen.Length) } else { $text }
    if ($new) {
        [IO.File]::AppendAllText($script:logCopy, $new, $utf8)
        $script:logAll += $new
    }
    $script:logSeen = $text
}
function Log-Has([string]$Pattern) { Sync-Log; return ($script:logAll -match $Pattern) }
function Log-Count([string]$Pattern) { Sync-Log; return ([regex]::Matches($script:logAll, $Pattern)).Count }
# log.txt lines that match, with the time of day instead of seconds since 1970.
function Log-Lines([string]$Pattern, [int]$Most = 200) {
    Sync-Log
    $found = @($script:logAll -split "`r?`n" | Where-Object { $_ -match $Pattern })
    $out = @()
    foreach ($l in @($found | Select-Object -Last $Most)) {
        if ($l -match '^(\d{9,11}) (.*)$') { $out += ("{0:HH:mm:ss} {1}" -f ([DateTimeOffset]::FromUnixTimeSeconds([int64]$Matches[1]).UtcDateTime), $Matches[2]) } else { $out += $l }
    }
    if ($found.Count -gt $Most) { $out = @("(the last $Most of $($found.Count) lines)") + $out }
    return $out
}
# A save of index.bin, seen from outside: the program writes index.tmp, syncs
# it and renames it over index.bin (src/index.rs, Index::save). Called about
# once a second; a save shorter than that shows only as a new write time.
function Poll-IndexSave {
    try {
        $now = Get-Date
        $tmp = Test-Path -LiteralPath (Join-Path $data 'index.tmp')
        $s = $script:save
        if ($tmp -and -not $s.TmpSince) { $s.TmpSince = $now }
        $item = Get-Item -LiteralPath $indexFile -ErrorAction SilentlyContinue
        $write = if ($item) { $item.LastWriteTimeUtc } else { $null }
        if ($write -and $s.LastWrite -and $write -ne $s.LastWrite) {
            $took = if ($s.TmpSince) { ($now - $s.TmpSince).TotalSeconds } else { 0.0 }
            $s.Count++
            if ($took -gt $s.Longest) { $s.Longest = $took }
            $line = "{0:HH:mm:ss} index.bin saved ({1:N0} MB), index.tmp was there for about {2:N0} s [{3}/{4}/{5}]" -f $now.ToUniversalTime(), ($item.Length / 1MB), $took, [MemSampler]::Version, [MemSampler]::Scenario, [MemSampler]::Phase
            $s.Lines.Add($line)
            Say "  $line"
            if (-not $tmp) { $s.TmpSince = $null } else { $s.TmpSince = $now }
        } elseif (-not $tmp -and $s.TmpSince) {
            $s.TmpSince = $null
        }
        if ($write) { $s.LastWrite = $write }
    } catch { }
}
# How long the save now under way (index.tmp present) has been running, in seconds; 0 when none.
function Save-Running { if ($script:save.TmpSince) { return ((Get-Date) - $script:save.TmpSince).TotalSeconds }; return 0.0 }
function Reset-SaveWatch { $script:save.TmpSince = $null; $script:save.LastWrite = $null; $script:save.Count = 0; $script:save.Longest = 0.0; $script:save.Lines.Clear() }

function Log-Tail([int]$Lines = 40) { Sync-Log; return @($script:logAll -split "`r?`n" | Where-Object { $_ } | Select-Object -Last $Lines) }

function Check-Crash([string]$When) {
    if (-not (Test-Path -LiteralPath $crashFile)) { return }
    $script:crashN++
    $text = ''
    try { $text = [MemSampler]::ReadShared($crashFile) } catch { $text = "(crash.txt could not be read: $_)" }
    $copy = Join-Path $Out ("crash-{0}-{1}-{2}-{3}.txt" -f $script:crashN, [MemSampler]::Version, [MemSampler]::Scenario, [MemSampler]::Phase)
    [IO.File]::WriteAllText($copy, $text, $utf8)
    $short = One-Line $text
    Crash ("crash.txt during {0} ({1} {2}/{3}): {4}" -f $When, [MemSampler]::Version, [MemSampler]::Scenario, [MemSampler]::Phase, $short.Substring(0, [Math]::Min(400, $short.Length)))
    # Removed so it is reported once, and the next start does not offer to send it.
    Remove-Item -LiteralPath $crashFile -Force -ErrorAction SilentlyContinue
}

function Stop-App {
    Get-Process bahth* -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    [void](Wait-Until { -not (Get-Process bahth* -ErrorAction SilentlyContinue) } 15)
    # WebView2 processes of the new look follow it; give them a moment.
    if (Get-Process msedgewebview2 -ErrorAction SilentlyContinue) { [void](Wait-Until { -not (Get-Process msedgewebview2 -ErrorAction SilentlyContinue) } 8) }
    Start-Sleep -Milliseconds 500
    $script:app = $null
}

# A first start, as a new user gets it: no data folder, English interface.
function Reset-Data {
    Stop-App
    for ($i = 0; $i -lt 6 -and (Test-Path -LiteralPath $data); $i++) {
        Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $data) { Start-Sleep -Seconds 1 }
    }
    if (Test-Path -LiteralPath $data) { Note "the data folder $data could not be removed completely" }
    Remove-Item 'HKCU:\Software\BahthSari' -Recurse -Force -ErrorAction SilentlyContinue
    New-Item 'HKCU:\Software\BahthSari' -Force | Out-Null
    Set-ItemProperty 'HKCU:\Software\BahthSari' -Name Language -Value 'en'
    $script:logSeen = ''
    $script:logAll = ''
    $script:optionsMenu = $null
    Reset-SaveWatch
}

# settings.txt as the program reads it (key=value, ASCII, no BOM). No
# first-run question (the box above the clock is on, as after "Yes"; no
# offer to move; no rating), and `update_check` a minute ago: the automatic
# check for a new version (src/update.rs, should_check_now) stays quiet for
# 24 hours, so the exe under test is not replaced. (A time in the future
# would start a check at once.)
function Write-Settings([hashtable]$Extra = @{}) {
    New-Item -ItemType Directory -Force $data | Out-Null
    $now = [int64]([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()) - 60
    $s = [ordered]@{ floating_asked = '1'; floating_mode = 'fade'; move_asked = '1'; rating_asked = '1'; new_ui = '0'; new_ui_failed = '0'; content_index = '1'; auto_index = '1'; tray_icon = '1'; update_check = "$now" }
    foreach ($k in @($Extra.Keys)) { $s[$k] = "$($Extra[$k])" }
    $text = ''
    foreach ($k in @($s.Keys)) { $text += "$k=$($s[$k])`n" }
    [IO.File]::WriteAllText($settingsFile, $text, (New-Object Text.ASCIIEncoding))
    Say "settings.txt: $($text -replace "`n", ' ')"
}

function File-Hash([string]$Path) { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }

function Copy-Exe([string]$From, [string]$To) {
    New-Item -ItemType Directory -Force (Split-Path $To -Parent) | Out-Null
    foreach ($suffix in @('.old', '.new')) { Remove-Item -LiteralPath ($To + $suffix) -Force -ErrorAction SilentlyContinue }
    for ($i = 0; $i -lt 10; $i++) {
        try { Copy-Item -LiteralPath $From -Destination $To -Force; return } catch { Start-Sleep -Seconds 1 }
    }
    Copy-Item -LiteralPath $From -Destination $To -Force
}
function Install-Exe([string]$V) {
    Copy-Exe $exes[$V] $instExe
    Say "installed $V at $instExe"
}

function Start-App([string[]]$AppArgs) {
    Say "starting $instExe $($AppArgs -join ' ')"
    if ($AppArgs -and $AppArgs.Count) { $script:app = Start-Process -FilePath $instExe -ArgumentList $AppArgs -WorkingDirectory $instDir -PassThru }
    else { $script:app = Start-Process -FilePath $instExe -WorkingDirectory $instDir -PassThru }
    Say "process $($script:app.Id)"
}

# ---------------------------------------------------------------- waiting
# Waits up to `Seconds` (or until `Until` is true) while the sampler runs.
# Every second a save of index.bin is noted; every 5 s log.txt is copied, a
# crash report or a program that is gone is noticed and an unexpected
# message box is answered. Every 30 s a line says what the program uses and
# what its status bar says.
function Wait-Sampled([int]$Seconds, [string]$What, [scriptblock]$Until = $null, [switch]$AllowDialogs, [switch]$AllowNoProcess) {
    $start = Get-Date
    $end = $start.AddSeconds($Seconds)
    $next = $start.AddSeconds(30)
    Say "waiting up to $Seconds s: $What"
    $turn = 0
    while ($true) {
        Poll-IndexSave
        if ($Until -and (& $Until)) { Say ("  done after {0:N0} s: {1}" -f ((Get-Date) - $start).TotalSeconds, $What); return $true }
        if (Time-Up) { Say "  the time budget is used up; stopped waiting for: $What"; break }
        if ($turn % 5 -eq 0) {
            Sync-Log
            Check-Crash $What
            if (-not $AllowNoProcess -and -not (Get-Process bahth -ErrorAction SilentlyContinue)) { throw "APP-EXITED: no bahth process any more while waiting for: $What" }
            if (-not $AllowDialogs) { Dismiss-Unexpected }
        }
        $turn++
        if ((Get-Date) -ge $next) {
            $next = (Get-Date).AddSeconds(30)
            Say ("  {0,5:N0} s  {1}  [{2}]" -f ((Get-Date) - $start).TotalSeconds, [MemSampler]::LastLine, (Status-Line))
        }
        $left = ($end - (Get-Date)).TotalSeconds
        if ($left -le 0) { break }
        Start-Sleep -Milliseconds ([int]([Math]::Min(1.0, $left) * 1000))
    }
    return $false
}

# Watches the bahth processes second by second (with a sample each second):
# who is there, who came, who went. Returns one entry per second.
function Observe([int]$Seconds, [string]$What, [scriptblock]$Stop = $null) {
    $start = Get-Date
    $timeline = New-Object System.Collections.Generic.List[object]
    $last = '?'
    while (((Get-Date) - $start).TotalSeconds -lt $Seconds) {
        [MemSampler]::SampleNow()
        $ids = @(Bahth-Ids)
        $t = ((Get-Date) - $start).TotalSeconds
        $timeline.Add([pscustomobject]@{ T = $t; Ids = $ids })
        $sig = $ids -join ', '
        if ($sig -ne $last) { Say ("  {0} +{1:N0} s: bahth processes [{2}]" -f $What, $t, $sig); $last = $sig }
        Sync-Log
        Check-Crash $What
        if ($Stop -and (& $Stop)) { break }
        if (Time-Up) { break }
        Start-Sleep -Milliseconds 900
    }
    return , $timeline.ToArray()
}
# First second a process id is seen in a timeline, or -1.
function First-Seen($Timeline, [scriptblock]$Match) {
    foreach ($e in $Timeline) { if (@($e.Ids | Where-Object $Match).Count) { return [double]$e.T } }
    return -1.0
}
# First second from which `Id` is never seen again, or -1 while it is still there at the end.
function Gone-At($Timeline, [int]$Id) {
    $gone = -1.0
    foreach ($e in $Timeline) {
        if ($e.Ids -contains $Id) { $gone = -1.0 } elseif ($gone -lt 0) { $gone = [double]$e.T }
    }
    return $gone
}

function Index-Ready { return ((Test-Path -LiteralPath $indexFile) -and (Log-Has 'index build: \d+ entries')) }
# The background content index is complete: its thread says so (a warm-up
# that was stopped by a rescan says "(stopped)" and starts again).
function Content-Settled { return (Log-Has 'content index warm: \d+ of \d+ new documents in \d+ ms on \d+ threads;') }

# Empty data folder, `V` installed, settings written, program started, name
# index awaited. `Label` is the version name in samples.csv.
function Start-Fresh([string]$V, [string]$Scenario, [string[]]$AppArgs, [hashtable]$Settings = @{}, [string]$Label = '') {
    if (-not $Label) { $Label = $V }
    Reset-Data
    Install-Exe $V
    Write-Settings $Settings
    $script:logCopy = Join-Path $logsDir ("{0}-{1}-log.txt" -f ($Label -replace '[^A-Za-z0-9.]+', '-'), $Scenario)
    Set-Phase $Label $Scenario 'index'
    $t = Get-Date
    Start-App $AppArgs
    $hidden = $AppArgs -contains '--startup'
    if (-not $hidden) {
        $main = Wait-Until { Main-Window } 40
        if (-not $main) { Dump-State "no main window ($V, $Scenario)"; throw "no main window 40 s after starting $V" }
        Say ("main window '{0}' after {1:N1} s" -f $main.Text, ((Get-Date) - $t).TotalSeconds)
    } else {
        Start-Sleep -Seconds 5
        if (-not (Get-Process bahth -ErrorAction SilentlyContinue)) { throw "APP-EXITED: $V did not stay running after --startup" }
        if (Main-Window) { Note "$V started with --startup shows a main window" }
    }
    Start-Sleep -Seconds 3
    Shot "$Label-$Scenario-start"
    $max = Plan-Seconds $script:indexMax "name index ($V, $Scenario)" 150 $script:indexMax
    $ok = Wait-Sampled $max "the name index of $V (scenario $Scenario)" { Index-Ready }
    $secs = ((Get-Date) - $t).TotalSeconds
    $entries = 0; $ms = 0
    if ($script:logAll -match 'index build: (\d+) entries in (\d+) ms') { $entries = [int64]$Matches[1]; $ms = [int64]$Matches[2] }
    if (-not $ok) { Note "$V / ${Scenario}: the name index was not ready after $max s; the scenario goes on with what there is" }
    $text = if ($ok) { "{0:N0} entries indexed in {1:N0} s" -f $entries, ($ms / 1000) } else { "name index NOT ready after {0:N0} s" -f $secs }
    Say "name index: $text"
    return [pscustomobject]@{ Ok = $ok; Seconds = $secs; Entries = $entries; Text = $text }
}

# ---------------------------------------------------------------- numbers
function Read-Samples {
    for ($i = 0; $i -lt 5; $i++) {
        try {
            $text = [MemSampler]::ReadShared($samplesFile)
            return @(($text -split "`r?`n") | Where-Object { $_ } | ConvertFrom-Csv)
        } catch { Start-Sleep -Milliseconds 300 }
    }
    return @()
}
function To-Num($V) { if ($null -eq $V -or "$V" -eq '') { return 0.0 }; return [double]::Parse("$V", [Globalization.CultureInfo]::InvariantCulture) }
# One entry per sample of the given phases: the bahth processes together.
function Get-Ticks([string]$Version, [string]$Scenario, [string[]]$Phases) {
    $mine = @(Read-Samples | Where-Object { $_.version -eq $Version -and $_.scenario -eq $Scenario -and ($Phases -contains $_.phase) })
    $ticks = New-Object System.Collections.Generic.List[object]
    foreach ($g in @($mine | Group-Object n)) {
        $first = $g.Group[0]
        $procs = @($g.Group | Where-Object { $_.pid -ne '0' })
        $t = [pscustomobject]@{
            N = [int64]$first.n; T = To-Num $first.elapsed_s; Phase = $first.phase; Count = $procs.Count
            Private = 0.0; PrivWs = 0.0; PrivWsKnown = $true; Ws = 0.0; OsPeak = 0.0; Handles = 0.0; Threads = 0.0; Gdi = 0.0; IndexSave = 0
            Wv2Count = [int](To-Num $first.wv2_count); Wv2Private = To-Num $first.wv2_private_mb; Ops = To-Num $first.churn_ops; ChurnMb = To-Num $first.churn_mb
            Ids = ($procs | ForEach-Object { $_.pid }) -join ' '
        }
        foreach ($p in $procs) {
            $t.Private += To-Num $p.private_mb; $t.Ws += To-Num $p.ws_mb
            if ("$($p.private_ws_mb)" -eq '') { $t.PrivWsKnown = $false } else { $t.PrivWs += To-Num $p.private_ws_mb }
            $t.OsPeak = [Math]::Max($t.OsPeak, (To-Num $p.peak_private_mb))
            $t.Handles += To-Num $p.handles; $t.Threads += To-Num $p.threads; $t.Gdi += To-Num $p.gdi
            if ("$($p.thread_names)" -match 'index-save=(\d+)') { $t.IndexSave += [int]$Matches[1] }
        }
        $ticks.Add($t)
    }
    return , @($ticks | Sort-Object N)
}
function Median($Values) {
    $v = @($Values | Where-Object { $null -ne $_ } | Sort-Object)
    if (-not $v.Count) { return [double]::NaN }
    return [double]$v[[int][Math]::Floor(($v.Count - 1) / 2)]
}
function Max-Of($Values) {
    $m = [double]::NaN
    foreach ($x in @($Values)) { if ([double]::IsNaN($m) -or $x -gt $m) { $m = [double]$x } }
    return $m
}
# Least-squares slope of y against x; NaN with fewer than 3 points or no spread.
function Slope($X, $Y) {
    $n = @($X).Count
    if ($n -lt 3 -or $n -ne @($Y).Count) { return [double]::NaN }
    $sx = 0.0; $sy = 0.0; $sxx = 0.0; $sxy = 0.0
    for ($i = 0; $i -lt $n; $i++) { $sx += $X[$i]; $sy += $Y[$i]; $sxx += $X[$i] * $X[$i]; $sxy += $X[$i] * $Y[$i] }
    $d = $n * $sxx - $sx * $sx
    if ([Math]::Abs($d) -lt 1e-9) { return [double]::NaN }
    return ($n * $sxy - $sx * $sy) / $d
}
# The longest time two or more bahth processes were alive together, in seconds.
function Longest-Pair($Ticks) {
    $best = 0.0; $from = -1.0
    foreach ($t in @($Ticks)) {
        if ($t.Count -ge 2) { if ($from -lt 0) { $from = $t.T }; $best = [Math]::Max($best, $t.T - $from) }
        else { $from = -1.0 }
    }
    return $best
}
# The last `Seconds` of a list of samples.
function Last-Seconds($Ticks, [double]$Seconds = 30) {
    $all = @($Ticks)
    if (-not $all.Count) { return @() }
    $end = $all[-1].T
    return @($all | Where-Object { $_.T -ge $end - $Seconds })
}

# One line of the result table: `Ticks` is the whole scenario, `Settle` the
# part whose end counts as settled (default: all of it).
function Add-Row([int]$Order, [string]$Scenario, [string]$Version, $Ticks, $Settle, [string]$Detail) {
    $all = @($Ticks)
    if (-not $Settle) { $Settle = $all }
    $tail = @(Last-Seconds $Settle 30)
    if (-not $all.Count -or -not $tail.Count) {
        Problem "no samples for '$Scenario' ($Version)"
        $rows.Add([pscustomobject]@{ Order = $Order; Scenario = $Scenario; Version = $Version; ProcMax = 'n/a'; ProcEnd = 'n/a'; PeakPrivateMB = ''; OsPeakPrivateMB = ''; SettledPrivateMB = ''; SettledPrivWsMB = ''; SettledWsMB = ''; Handles = ''; Threads = ''; Gdi = ''; Wv2Count = 0; Wv2PrivateMB = ''; PairSeconds = 0; Detail = "no samples. $Detail"; Leg = $Leg })
        return
    }
    $pair = Longest-Pair $all
    $privWs = if (@($tail | Where-Object { -not $_.PrivWsKnown }).Count) { '' } else { [Math]::Round((Median ($tail | ForEach-Object { $_.PrivWs })), 1) }
    $row = [pscustomobject]@{
        Order = $Order; Scenario = $Scenario; Version = $Version
        ProcMax = [int](Max-Of ($all | ForEach-Object { $_.Count })); ProcEnd = $tail[-1].Count
        PeakPrivateMB = [Math]::Round((Max-Of ($all | ForEach-Object { $_.Private })), 1)
        OsPeakPrivateMB = [Math]::Round((Max-Of ($all | ForEach-Object { $_.OsPeak })), 1)
        SettledPrivateMB = [Math]::Round((Median ($tail | ForEach-Object { $_.Private })), 1)
        SettledPrivWsMB = $privWs
        SettledWsMB = [Math]::Round((Median ($tail | ForEach-Object { $_.Ws })), 1)
        Handles = [int](Median ($tail | ForEach-Object { $_.Handles })); Threads = [int](Median ($tail | ForEach-Object { $_.Threads })); Gdi = [int](Median ($tail | ForEach-Object { $_.Gdi }))
        Wv2Count = [int](Max-Of ($tail | ForEach-Object { $_.Wv2Count })); Wv2PrivateMB = [Math]::Round((Median ($tail | ForEach-Object { $_.Wv2Private })), 1)
        PairSeconds = [int]$pair; Detail = $Detail; Leg = $Leg
    }
    $rows.Add($row)
    Say ("RESULT {0} [{1}]: processes max {2} / end {3}; private peak {4} MB (OS peak {5}), settled {6} MB; private WS {7}; working set {8} MB. {9}" -f $Scenario, $Version, $row.ProcMax, $row.ProcEnd, $row.PeakPrivateMB, $row.OsPeakPrivateMB, $row.SettledPrivateMB, $row.SettledPrivWsMB, $row.SettledWsMB, $Detail)
    if ($row.SettledPrivateMB -gt 1024) { Warn ("{0} [{1}]: settled private bytes are {2:N0} MB (over 1 GB); peak {3:N0} MB" -f $Scenario, $Version, $row.SettledPrivateMB, $row.PeakPrivateMB) }
    if ($pair -gt 60) { Warn ("{0} [{1}]: two or more bahth processes stayed alive together for {2:N0} s" -f $Scenario, $Version, $pair) }
}

# ---------------------------------------------------------------- Options menu
# Opens the classic window's Options menu and chooses the item whose text
# matches, with the mouse as a person does. Returns true when an item was
# chosen (the caller checks that it had its effect).
function Invoke-OptionsItem([string]$Pattern) {
    $main = Classic-Window
    if (-not $main) { Note "Options menu: no classic window on screen"; return $false }
    $ui = Find-Controls $main
    if (-not $ui.Options) { Note 'Options menu: no Options button'; return $false }
    Focus-Window $main.Handle
    $menu = $null
    for ($try = 1; $try -le 3 -and -not $menu; $try++) {
        [void][MemUi]::PostMessage($ui.Options.Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
        $menu = Wait-Until { Open-Menu } 6
    }
    if (-not $menu) { Note 'Options menu: no popup menu window (#32768) after clicking Options'; Shot 'options-not-open'; return $false }
    $items = @([MemUi]::MenuItems($menu.Handle))
    Say ("Options items: " + (($items | Where-Object { $_ }) -join ' / '))
    $idx = -1
    for ($i = 0; $i -lt $items.Count; $i++) { if ($items[$i] -match $Pattern) { $idx = $i; break } }
    $script:optionsMenu = [pscustomobject]@{ Menu = [MemUi]::MenuHandle($menu.Handle); Items = $items; Window = $main.Handle }
    if ($idx -lt 0) { Note "Options menu: no item matches '$Pattern'"; [MemUi]::Chord($VK.ESC); return $false }
    Shot 'options-menu'
    $r = [MemUi]::MenuItemRect($menu.Handle, $idx)
    if ($r) {
        Say ("clicking '{0}' at {1},{2}" -f $items[$idx], [int](($r[0] + $r[2]) / 2), [int](($r[1] + $r[3]) / 2))
        [MemUi]::Click([int](($r[0] + $r[2]) / 2), [int](($r[1] + $r[3]) / 2), $false)
        Start-Sleep -Milliseconds 800
    } else { Say 'no rectangle for the menu item' }
    if (Open-Menu) {
        # The click did not land: close the menu and send what choosing the item sends.
        [MemUi]::Chord($VK.ESC)
        Start-Sleep -Milliseconds 400
        Note "Options menu: the mouse click did not choose '$($items[$idx])'; sent WM_MENUCOMMAND instead"
        [void][MemUi]::PostMessage($main.Handle, $WM_MENUCOMMAND, [IntPtr]$idx, $script:optionsMenu.Menu)
    }
    return $true
}
# The same choice without the mouse, from the menu seen by Invoke-OptionsItem.
function Post-OptionsItem([string]$Pattern) {
    $m = $script:optionsMenu
    if (-not $m) { return $false }
    for ($i = 0; $i -lt $m.Items.Count; $i++) {
        if ($m.Items[$i] -match $Pattern) {
            [void][MemUi]::PostMessage($m.Window, $WM_MENUCOMMAND, [IntPtr]$i, $m.Menu)
            return $true
        }
    }
    return $false
}

# How long "Exit the program" takes with the index this run has built: the
# updater's new copy waits at most a minute for the old one to go. The
# window is brought back the way the program tells its users to: by
# starting it again. Never fails the run.
function Exit-Probe([string]$Label, [string]$Scenario) {
    try {
        Set-Phase $Label $Scenario 'exit'
        if (-not (Get-Process bahth -ErrorAction SilentlyContinue)) { return 'not running' }
        $before = @(Bahth-Ids)
        if (-not (Classic-Window)) {
            $second = Start-Process -FilePath $instExe -WorkingDirectory $instDir -PassThru
            $main = Wait-Until { Classic-Window } 20
            [void]$second.WaitForExit(15000)
            $handed = if ($second.HasExited) { 'the second start handed over and exited' } else { 'the SECOND START IS STILL RUNNING after 15 s' }
            Say "starting it again while it runs: window shown = $([bool]$main); $handed; processes: $((Bahth-Ids) -join ', ')"
            Detail "- Started again while running in the tray: window shown = $([bool]$main); $handed."
            if (-not $main) { return 'window not shown by a second start' }
        }
        if (-not (Invoke-OptionsItem 'Exit the program|خروج من البرنامج')) { return 'Exit item not reached' }
        $t = Get-Date
        $gone = Wait-Until { -not (Get-Process bahth -ErrorAction SilentlyContinue) } (Plan-Seconds 120 'exit probe' 60 60)
        if (-not $gone -and (Post-OptionsItem 'Exit the program|خروج من البرنامج')) {
            $gone = Wait-Until { -not (Get-Process bahth -ErrorAction SilentlyContinue) } 60
        }
        $secs = ((Get-Date) - $t).TotalSeconds
        $text = if ($gone) { "exit from the Options menu took {0:N1} s" -f $secs } else { "STILL RUNNING {0:N0} s after Exit from the Options menu" -f $secs }
        Say $text
        return $text
    } catch { return "exit probe failed: $_" }
}

# What a run leaves: sizes, the exe untouched, the end of log.txt.
function Finish-Session([string]$Label, [string]$Scenario, [string]$Title, [string[]]$Facts, [string]$ExpectHash = '') {
    Sync-Log
    Check-Crash "end of $Scenario"
    Detail "### $Title"
    Detail ''
    foreach ($f in @($Facts)) { if ($f) { Detail "- $f" } }
    try {
        $idx = if (Test-Path -LiteralPath $indexFile) { (Get-Item -LiteralPath $indexFile).Length / 1MB } else { 0 }
        $content = 0
        $cdir = Join-Path $data 'content'
        if (Test-Path -LiteralPath $cdir) { foreach ($f in @(Get-ChildItem -LiteralPath $cdir -File -ErrorAction SilentlyContinue)) { $content += $f.Length / 1MB } }
        Detail ("- On disk: index.bin {0:N0} MB, content index {1:N0} MB." -f $idx, $content)
        $updates = @($script:logAll -split "`r?`n" | Where-Object { $_ -match ' update: ' })
        if ($ExpectHash) {
            $now = File-Hash $instExe
            if ($now -ne $ExpectHash) { Problem "${Title}: the exe under test was replaced during the run (SHA-256 $now)" }
            elseif ($updates.Count) { Note "${Title}: the log has update lines although the check was held back: $($updates -join ' / ')" }
            else { Detail '- The exe was not replaced and no update check ran.' }
        }
        foreach ($l in @(Process-List)) { Detail "- Process at the end: ``$l``" }
    } catch { Note "finish ${Title}: $_" }
    Detail ''
    Detail '<details><summary>log.txt (last 40 lines)</summary>'
    Detail ''
    Detail '```text'
    foreach ($l in @(Log-Tail 40)) { Detail $l }
    Detail '```'
    Detail '</details>'
    Detail ''
    Say "--- log.txt, last 40 lines ($Title)"
    foreach ($l in @(Log-Tail 40)) { Write-Host "    $l" }
    Stop-App
}

function Run-Scenario([string]$Name, [scriptblock]$Body) {
    Say "==================== $Name"
    if ((Time-Up) -or (Time-Left) -lt 300) { Note "$Name was skipped: the time budget of $BudgetMinutes min is used up"; return }
    try { & $Body }
    catch {
        $m = One-Line "$_"
        $at = $_.InvocationInfo.ScriptLineNumber
        if ($m -like 'APP-EXITED*') { Crash "${Name}: $m" } else { Problem "${Name}: $m (line $at)" }
        Shot "error-$Name"
        Dump-State "error in $Name"
        try { [MemGen]::StopChurn() } catch { }
    }
    finally {
        try { Sync-Log; Check-Crash $Name } catch { }
        try { Stop-App } catch { }
    }
}

# ---------------------------------------------------------------- synthetic data
function Free-GB([string]$Path) {
    $root = [IO.Path]::GetPathRoot($Path)
    return ([IO.DriveInfo]$root).AvailableFreeSpace / 1GB
}

# How much of the synthetic data fits: -SizeScale, less when the disk is short.
function Data-Scale([double]$NeedGB, [string]$What) {
    $free = Free-GB $dataRoot
    $scale = $SizeScale
    $room = $free - 12
    $fits = [Math]::Max(0.02, $room / $NeedGB)
    if ($fits -lt $scale) {
        $scale = $fits
        Warn ("{0} scaled down to {1:P0}: {2:N0} GB free on {3}, {4:N1} GB needed plus 12 GB kept free" -f $What, $scale, $free, $dataRoot, ($NeedGB * $SizeScale))
    }
    return $scale
}

# The deep tree of small files (scenarios B and F), made once per run of this script.
function New-SmallTree {
    if ($script:tree) { return $script:tree }
    $root = Join-Path $dataRoot 'heavy'
    $scale = Data-Scale 1.5 'the tree of small files'
    $want = [int](500000 * $scale)
    Say ("small files in {0}: {1:N0} wanted ({2:N0} GB free)" -f $root, $want, (Free-GB $dataRoot))
    $t = Get-Date
    $count = 0
    try { $count = [MemGen]::SmallTree($root, $want, (Plan-Seconds 600 'small files of the heavy tree')) } catch { Problem "heavy tree: small files FAILED: $_" }
    $text = "{0:N0} of {1:N0} small files (Unity Library and node_modules shapes) under {2} in {3:N0} s = {4:N0} files/s" -f $count, $want, $root, ((Get-Date) - $t).TotalSeconds, ($count / [Math]::Max(1.0, ((Get-Date) - $t).TotalSeconds))
    Say "  $text"
    if ([MemGen]::Errors -gt 0) { Note "heavy tree: $([MemGen]::Errors) errors while writing the small files, last: $([MemGen]::LastError)" }
    if ($count -lt $want) { Note "heavy tree: only $count of $want small files were made in the time allowed" }
    Detail '### Tree of small files (scenarios B and F)'
    Detail ''
    Detail "- $text"
    Detail ''
    $script:tree = [pscustomobject]@{ Root = $root; Scale = $scale; SmallFiles = $count; Text = $text }
    return $script:tree
}

# Scenario B's big files, made once per run of this script.
function New-BigFiles {
    if ($script:bigFiles) { return $script:bigFiles }
    $root = Join-Path $dataRoot 'heavy'
    # GB at -SizeScale 1: the big files and the files near the limits.
    $scale = Data-Scale 12.0 'the big files of the heavy tree'
    Say ("big files in {0} at scale {1:N2} ({2:N0} GB free)" -f $root, $scale, (Free-GB $dataRoot))
    $made = New-Object System.Collections.Generic.List[string]
    $t0 = Get-Date
    $mb = { param([double]$m) [int64]([Math]::Max(1.0, $m * $scale) * 1MB) }
    $step = {
        param([string]$Name, [scriptblock]$Do)
        $t = Get-Date
        try {
            $bytes = & $Do
            $line = "{0}: {1:N0} MB in {2:N1} s" -f $Name, ($bytes / 1MB), ((Get-Date) - $t).TotalSeconds
        } catch { $line = "$Name FAILED: $_"; Problem "heavy tree: $line" }
        Say "  $line"
        $made.Add($line)
    }
    $big = Join-Path $root 'big'
    $limit = Join-Path $root 'limits'
    # Far over the content indexer's 100 MB file limit: must be skipped, not read.
    & $step 'big\notes-2gb.txt' { [MemGen]::BigText((Join-Path $big 'notes-2gb.txt'), (& $mb 2048), 0, 1) }
    & $step 'big\scene-export-1.5gb.json' { [MemGen]::BigText((Join-Path $big 'scene-export-1.5gb.json'), (& $mb 1536), 1, 2) }
    & $step 'big\editor-1gb.log' { [MemGen]::BigText((Join-Path $big 'editor-1gb.log'), (& $mb 1024), 2, 3) }
    & $step 'big\table-700mb.csv' { [MemGen]::BigText((Join-Path $big 'table-700mb.csv'), (& $mb 700), 3, 4) }
    # 4,000 small text members, one 512 MB text member (deflated) and a stored one of 830 MB.
    & $step 'big\assets-1gb.zip' { [MemGen]::Zip((Join-Path $big 'assets-1gb.zip'), [int](4000 * [Math]::Min(1.0, [Math]::Max(0.05, $scale))), 48KB, (& $mb 512), (& $mb 830), 5) }
    & $step 'big\backup.tar.gz' { [MemGen]::TarGz((Join-Path $big 'backup.tar.gz'), [int](3000 * [Math]::Min(1.0, [Math]::Max(0.05, $scale))), 32KB, (& $mb 800), 6) }
    & $step 'big\project.blend' { [MemGen]::BigBinary((Join-Path $big 'project.blend'), (& $mb 1024), 'BLENDER-v300SYNTHETIC', 7) }
    & $step 'big\model.fbx' { [MemGen]::BigBinary((Join-Path $big 'model.fbx'), (& $mb 1024), 'Kaydara FBX Binary  SYNTHETIC', 8) }
    & $step 'big\level.asset' { [MemGen]::BigBinary((Join-Path $big 'level.asset'), (& $mb 1024), "%YAML 1.1`n%TAG !u! tag:unity3d.com,2011:`n--- synthetic`n", 9) }
    & $step 'big\pack.unitypackage' { [MemGen]::BigBinary((Join-Path $big 'pack.unitypackage'), (& $mb 1024), 'SYNTHETIC-UNITYPACKAGE', 10) }
    # Just under the limits (100 MB per file, 8 MB "big file", 2 MB "big
    # archive"): these the content indexer does read.
    for ($i = 1; $i -le 3; $i++) { $n = $i; & $step "limits\near-100mb-$n.txt" { [MemGen]::BigText((Join-Path $limit "near-100mb-$n.txt"), (& $mb 99), 0, 20 + $n) } }
    & $step 'limits\near-100mb.json' { [MemGen]::BigText((Join-Path $limit 'near-100mb.json'), (& $mb 95), 1, 24) }
    & $step 'limits\near-100mb.log' { [MemGen]::BigText((Join-Path $limit 'near-100mb.log'), (& $mb 95), 2, 25) }
    & $step 'limits\near-100mb.csv' { [MemGen]::BigText((Join-Path $limit 'near-100mb.csv'), (& $mb 95), 3, 26) }
    & $step 'limits\mid-NN (40 files of up to 7.9 MB)' {
        $sum = 0
        for ($i = 0; $i -lt 40; $i++) { $ext = @('.txt', '.json', '.log', '.csv')[$i % 4]; $sum += [MemGen]::BigText((Join-Path $limit ("mid-{0:D2}{1}" -f $i, $ext)), [int64]([Math]::Min(1.0, [Math]::Max(0.05, $scale)) * 7.9MB), ($i % 4), 30 + $i) }
        $sum
    }
    & $step 'limits\members.zip (3,000 members, under 100 MB)' { [MemGen]::Zip((Join-Path $limit 'members.zip'), 3000, 32KB, [int64]([Math]::Min(1.0, $scale) * 200MB), 0, 11) }
    & $step 'limits\logs.tar.gz (1,500 members, under 100 MB)' { [MemGen]::TarGz((Join-Path $limit 'logs.tar.gz'), 1500, 32KB, [int64]([Math]::Min(1.0, $scale) * 150MB), 12) }
    if ([MemGen]::Errors -gt 0) { Note "heavy tree: $([MemGen]::Errors) errors while writing, last: $([MemGen]::LastError)" }
    $text = "big files under {0}: made in {1:N0} s at scale {2:N2}; {3:N0} GB free afterwards" -f $root, ((Get-Date) - $t0).TotalSeconds, $scale, (Free-GB $dataRoot)
    Say $text
    Detail '### Big files (scenario B)'
    Detail ''
    Detail "- $text"
    foreach ($l in $made) { Detail "- $l" }
    Detail ''
    $script:bigFiles = [pscustomobject]@{ Root = $root; Scale = $scale; Text = $text }
    return $script:bigFiles
}

# ---------------------------------------------------------------- A: baseline
function Scenario-A([string]$V) {
    $hash = File-Hash $exes[$V]
    $s = Start-Fresh $V 'A' @()
    Set-Phase $V 'A' 'idle'
    [void](Wait-Sampled (Plan-Seconds 180 'A idle') "A: three minutes after the name index ($V)")
    $status = Status-Line
    $all = Get-Ticks $V 'A' @('index', 'idle')
    $idle = Get-Ticks $V 'A' @('idle')
    Add-Row 10 'A. Baseline: this machine as it is, 3 min after the name index' $V $all $idle "$($s.Text); status bar at the end: $status"
    Shot "$V-A-end"
    Finish-Session $V 'A' "A. Baseline, $V" @("Name index: $($s.Text).", "Status bar at the end: $status", "Sampler at the end: $([MemSampler]::LastLine)") $hash
}

# ---------------------------------------------------------------- C: search stress
function Type-Query([string]$Q, [int]$Delay = 8) {
    [MemUi]::Chord($VK.CTRL, $VK.A)
    [MemUi]::Chord($VK.BACK)
    Start-Sleep -Milliseconds 60
    [MemUi]::Type($Q, $Delay)
}
function Now-Private { [MemSampler]::SampleNow(); return [Math]::Round([MemSampler]::LastPrivateMb, 0) }

function Do-SearchStress([string]$V) {
    Set-Phase $V 'C' 'before'
    $main = Classic-Window
    if (-not $main) { Dump-State 'C: no classic window'; throw 'C: the classic window is not on screen' }
    $ui = Find-Controls $main
    if (-not $ui.Query -or -not $ui.List) { Dump-State 'C: controls'; throw 'C: no search box or result list in the main window' }
    [void](Wait-Sampled (Plan-Seconds 30 'C before') "C: before the searches ($V)")
    $before = Now-Private

    Set-Phase $V 'C' 'search'
    # Match almost everything.
    $broad = @('e', '.', 'a', '*', 'e a', 'ext:dll', 'size:>0', '1')
    # Long and odd ones.
    $odd = @(
        ('x' * 250), 'ـــــــ', '"a b"~3', 'regex:.*', '😀😀😀 emoji', ('تقرير مالي ' * 20), '*.*.*.*.*', '((((((((',
        ([string][char]0x202E + 'abc' + [char]0x200F + [char]0x200B), 'C:\Users\*\..\..\*', 'size:>999999999999999999999', ('a ' * 120)
    )
    $rounds = [int][Math]::Max(2, [Math]::Round(20 * [Math]::Min(1.0, $TimeScale)))
    $popups = $rounds
    $maxRows = 0; $reached = 0; $contentRuns = 0
    $t = Get-Date
    for ($r = 1; $r -le $rounds; $r++) {
        if (-not (Get-Process bahth -ErrorAction SilentlyContinue)) { throw "APP-EXITED: the program went away in search round $r" }
        if ((Time-Up) -or (Time-Left) -lt 420) { Note "C ($V): stopped after $($r - 1) of $rounds rounds, the time budget is nearly used up"; $rounds = $r - 1; break }
        Dismiss-Unexpected
        Focus-Window $ui.Main.Handle
        Click-Center $ui.Query
        Start-Sleep -Milliseconds 150
        $queries = @($broad) + @($odd[($r - 1) % $odd.Count], $odd[($r + 4) % $odd.Count])
        foreach ($q in $queries) {
            Type-Query $q
            Start-Sleep -Milliseconds 900
            $n = [MemUi]::ListCount($ui.List.Handle)
            if ($n -gt $maxRows) { $maxRows = $n }
            if ($r -eq 1) {
                $shown = [MemUi]::Text($ui.Query.Handle)
                if ($shown -eq $q) { $reached++ }
                Say ("  '{0}' -> {1} rows (the box holds {2} characters)" -f $q.Substring(0, [Math]::Min(30, $q.Length)), $n, $shown.Length)
            }
        }
        # A paste of about 5,000 characters, then one more key.
        [void][MemUi]::SetText($ui.Query.Handle, ('long query ' * 450))
        [MemUi]::Type('x', 0)
        Start-Sleep -Milliseconds 900
        if ($r -eq 1) {
            Shot "$V-C-round1"
            if ($reached -eq 0) { Problem "C ($V): the typed searches did not reach the search box; the search numbers of C mean little" }
        }
        # Every fifth round: a search inside documents for a common word.
        if (($r % 5 -eq 0 -or ($rounds -lt 5 -and $r -eq $rounds)) -and $ui.Content) {
            [void][MemUi]::PostMessage($ui.Content.Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
            Start-Sleep -Milliseconds 500
            $on = [MemUi]::Send($ui.Content.Handle, $BM_GETCHECK, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
            Type-Query 'the'
            [MemUi]::Chord($VK.ENTER)
            Start-Sleep -Seconds (Plan-Seconds 20 'C content search')
            $contentRuns++
            Say ("  search inside documents (box ticked: {0}): {1} rows; status: {2}; {3}" -f $on, [MemUi]::ListCount($ui.List.Handle), (Status-Line), [MemSampler]::LastLine)
            if ($contentRuns -eq 1) { Shot "$V-C-content" }
            for ($i = 0; $i -lt 2; $i++) {
                if ([MemUi]::Send($ui.Content.Handle, $BM_GETCHECK, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -eq 1) {
                    [void][MemUi]::PostMessage($ui.Content.Handle, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
                    Start-Sleep -Milliseconds 500
                }
            }
        }
        Sync-Log
        Check-Crash "C round $r"
        Say ("C round {0}/{1} after {2:N0} s: {3}" -f $r, $rounds, ((Get-Date) - $t).TotalSeconds, [MemSampler]::LastLine)
    }
    Focus-Window $ui.Main.Handle
    Click-Center $ui.Query
    Type-Query ''
    $afterSearch = Now-Private

    Set-Phase $V 'C' 'popup'
    $opened = 0; $missed = 0
    $words = @('e', 'report', 'تقرير', 'a', '2+2', 'notepad')
    for ($i = 1; $i -le $popups; $i++) {
        if (Time-Up) { break }
        [MemUi]::Chord($VK.CTRL, $VK.ALT, $VK.F)
        $pop = Wait-Until { Popup-Window } 4
        if ($pop) {
            $opened++
            [MemUi]::Type($words[$i % $words.Count], 15)
            Start-Sleep -Milliseconds 900
            if ($i -eq 1) { Shot "$V-C-popup" }
            # Esc only while the popup is there: in the main window it hides the window.
            [MemUi]::Chord($VK.ESC)
            [void](Wait-Until { -not (Popup-Window) } 3)
        } else {
            $missed++
            if ($missed -ge 3 -and $opened -eq 0) { Note "C ($V): Ctrl+Alt+F did not open the popup (3 tries); skipped the popup part"; break }
        }
        Start-Sleep -Milliseconds 300
    }
    Check-Crash 'C popup'
    $afterPopup = Now-Private

    Set-Phase $V 'C' 'after'
    [void](Wait-Sampled (Plan-Seconds 300 'C idle after the searches') "C: five minutes idle after the searches ($V)")
    $all = Get-Ticks $V 'C' @('before', 'search', 'popup', 'after')
    $after = Get-Ticks $V 'C' @('after')
    $end = Median (@(Last-Seconds $after 30) | ForEach-Object { $_.Private })
    $peak = Max-Of ($all | ForEach-Object { $_.Private })
    $text = "private {0:N0} MB before, peak {1:N0} MB, {2:N0} MB right after {9} rounds, {3:N0} MB after the popups, {4:N0} MB after the idle time ({5:+0;-0;0} MB against before); most rows shown {6:N0}; popup opened {7}/{10}; searches inside documents {8}" -f $before, $peak, $afterSearch, $afterPopup, $end, ($end - $before), $maxRows, $opened, $contentRuns, $rounds, $popups
    Add-Row 30 'C. Search stress: 20 rounds of broad and odd searches, 20 popups, then 5 min idle' $V $all $after $text
    return "C: $text."
}

# ---------------------------------------------------------------- E: long idle in the tray
function Do-TrayIdle([string]$V) {
    Set-Phase $V 'E' 'hide'
    $main = Main-Window
    if ($main) {
        # What the title bar's X sends: the window hides, the program stays.
        [void][MemUi]::PostMessage($main.Handle, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        Start-Sleep -Seconds 3
    }
    if (-not (Get-Process bahth -ErrorAction SilentlyContinue)) { throw 'APP-EXITED: closing the window ended the program instead of hiding it to the tray' }
    if (Main-Window) { Note "E ($V): the main window is still on screen after the close button" }
    Set-Phase $V 'E' 'tray'
    [void](Wait-Sampled (Plan-Seconds 600 'E idle in the tray') "E: ten minutes in the tray ($V)")
    $ticks = Get-Ticks $V 'E' @('tray')
    $x = @($ticks | ForEach-Object { $_.T / 60.0 })
    $y = @($ticks | ForEach-Object { $_.Private })
    $slope = Slope $x $y
    $first = if ($y.Count) { $y[0] } else { [double]::NaN }
    $last = if ($y.Count) { $y[-1] } else { [double]::NaN }
    $mins = if ($x.Count) { $x[-1] - $x[0] } else { 0 }
    $text = "slope of private bytes {0:+0.00;-0.00;0.00} MB/min over {1:N1} min in the tray ({2:N0} MB at the start, {3:N0} MB at the end); status bar: {4}" -f $slope, $mins, $first, $last, (Status-Line)
    Add-Row 50 'E. Long idle: 10 min in the tray (same process as B and C)' $V $ticks $ticks $text
    return "E: $text."
}

# ---------------------------------------------------------------- B (then C, E): the heavy tree
function Scenario-Static([string]$V, [bool]$DoB, [bool]$DoC, [bool]$DoE) {
    $scn = if ($DoB) { 'B' } elseif ($DoC) { 'C' } else { 'E' }
    $tree = $null; $bigFiles = $null
    if ($DoB) { $tree = New-SmallTree; $bigFiles = New-BigFiles }
    $hash = File-Hash $exes[$V]
    $s = Start-Fresh $V $scn @()
    $facts = New-Object System.Collections.Generic.List[string]
    $facts.Add("Name index: $($s.Text).")
    $last = $scn
    if ($DoB) {
        Set-Phase $V 'B' 'content'
        $t = Get-Date
        $max = Plan-Seconds 1200 'B content indexing'
        $settled = Wait-Sampled $max "background content indexing to settle ($V)" { Content-Settled }
        $secs = ((Get-Date) - $t).TotalSeconds
        $warm = @($script:logAll -split "`r?`n" | Where-Object { $_ -match 'content index warm:' } | Select-Object -Last 1) -join ''
        $contentText = if ($settled) { "content indexing settled {0:N0} s after the name index ({1})" -f $secs, ($warm -replace '^\d+ ', '') } else { "content indexing still running after the {0:N0} s time box (status bar: {1})" -f $secs, (Status-Line) }
        Say $contentText
        Set-Phase $V 'B' 'settled'
        [void](Wait-Sampled (Plan-Seconds 90 'B settled') "B: after content indexing ($V)")
        $all = Get-Ticks $V 'B' @('index', 'content', 'settled')
        $end = Get-Ticks $V 'B' @('settled')
        Add-Row 20 'B. Heavy tree: name index, then background content indexing' $V $all $end "$($s.Text); $contentText; $($tree.SmallFiles) small files, big files at scale $([Math]::Round($bigFiles.Scale, 2))"
        $facts.Add("B: $contentText.")
        Shot "$V-B-end"
    }
    if ($DoC) { $facts.Add((Do-SearchStress $V)); $last = 'C' }
    if ($DoE) { $facts.Add((Do-TrayIdle $V)); $last = 'E' }
    $facts.Add("Exit: $(Exit-Probe $V $last).")
    $names = @(); if ($DoB) { $names += 'B' }; if ($DoC) { $names += 'C' }; if ($DoE) { $names += 'E' }
    Finish-Session $V $scn "$($names -join ', '), one run of $V" $facts.ToArray() $hash
}

# ---------------------------------------------------------------- F: churn
# F: the saves are made slow the natural way, for any exe: every core busy
# with normal-priority work (the program saves on a below-normal thread) and
# the data folder's disk busy with unbuffered writes.
# H (-Hook): the saves are delayed by the test hook of the branch build (an
# environment variable, -SaveDelayVar = -SaveDelayMs); nothing else is loaded.
function Scenario-F([string]$V, [bool]$Hook = $false) {
    $hash = File-Hash $exes[$V]
    $scn = if ($Hook) { 'H' } else { 'F' }
    $burners = if ($Hook) { 0 } elseif ($CpuBurners -lt 0) { [Environment]::ProcessorCount } else { $CpuBurners }
    $loaders = if ($Hook) { 0 } else { $DiskLoadThreads }
    # A big index: the machine's own files plus the tree of small files.
    $tree = New-SmallTree
    # As Windows starts it at sign-in: hidden, in the tray.
    if ($Hook) { Set-Item -Path "Env:$SaveDelayVar" -Value "$SaveDelayMs"; Say "hook: $SaveDelayVar=$SaveDelayMs for this start" }
    try { $s = Start-Fresh $V $scn @('--startup') }
    finally { if ($Hook) { Remove-Item -Path "Env:$SaveDelayVar" -ErrorAction SilentlyContinue } }
    Set-Phase $V $scn 'pre'
    [void](Wait-Sampled (Plan-Seconds 120 'F before the churn') "${scn}: in the tray, the idle baseline before the churn ($V)")
    $pre = Get-Ticks $V $scn @('pre')
    $root = Join-Path $dataRoot 'churn'
    [void](Remove-Tree $root 120)
    $free = Free-GB $dataRoot
    # The three downloads grow to this size each, then start again.
    $cap = [int64][Math]::Max(256MB, [Math]::Min(3GB * $SizeScale, ($free - 14) * 1GB / 4))
    if ($free -lt 20) { Warn ("$scn ({0}): only {1:N0} GB free on {2}; the growing files are capped at {3:N0} MB each" -f $V, $free, $dataRoot, ($cap / 1MB)) }
    $churnSecs = Plan-Seconds 900 'F churn' 800
    Say ("churn in {0} for {1} s: 3 renderers at 15 MB/s, 2 Library threads at 350 files/s, downloads at 30 MB/s growing to {2:N0} MB, documents at 40 saves/s; {3:N0} GB free" -f $root, $churnSecs, ($cap / 1MB), $free)

    # The disk that holds the data folder (index.bin) is kept busy too.
    $loadDir = Join-Path $env:LOCALAPPDATA 'InstaFind-memtest-load'
    $loadFree = Free-GB $loadDir
    $loadCap = [int64][Math]::Max(256MB, [Math]::Min(4GB * $SizeScale, ($loadFree - 16) * 1GB / 4))
    Say ("load: {0} CPU burner threads at normal priority; {1} threads of unbuffered sequential writes in {2} (files of {3:N0} MB, {4:N0} GB free there)" -f $burners, $loaders, $loadDir, ($loadCap / 1MB), $loadFree)
    Reset-SaveWatch
    Poll-IndexSave

    Set-Phase $V $scn 'churn'
    [MemGen]::StartChurn($root, $cap, 3, 15.0, 2, 350, 30.0, 40)
    if ($loaders -gt 0) { [MemGen]::StartDiskLoad($loadDir, $loaders, $loadCap) }
    if ($burners -gt 0) { [MemGen]::StartCpuBurners($burners) }
    $t = Get-Date
    $lastOps = 0.0; $lastMb = 0.0; $lastLoad = 0.0; $lastT = $t; $nextLine = $t.AddSeconds(15); $nextCheck = $t
    $stopNote = ''
    try {
        while (((Get-Date) - $t).TotalSeconds -lt $churnSecs -and -not (Time-Up)) {
            Start-Sleep -Seconds 1
            Poll-IndexSave
            if ((Get-Date) -lt $nextCheck) { continue }
            $nextCheck = (Get-Date).AddSeconds(5)
            Sync-Log
            Check-Crash "$scn churn"
            if (-not (Get-Process bahth -ErrorAction SilentlyContinue)) { throw 'APP-EXITED: the program went away during the churn' }
            Dismiss-Unexpected
            if ((Get-Date) -ge $nextLine) {
                $nextLine = (Get-Date).AddSeconds(15)
                $ops = [double][MemGen]::Ops; $mb = [MemGen]::Bytes / 1MB; $load = [MemGen]::LoadBytes / 1MB
                $dt = [Math]::Max(0.1, ((Get-Date) - $lastT).TotalSeconds)
                Say ("  churn {0,4:N0} s: {1:N0} file operations ({2:N0}/s now), {3:N0} MB written ({4:N0} MB/s now), disk load {5:N0} MB/s, {6} generator errors, {7:N0} GB free, saves of index.bin seen {8} (one running for {9:N0} s) | {10} | {11}" -f ((Get-Date) - $t).TotalSeconds, $ops, (($ops - $lastOps) / $dt), $mb, (($mb - $lastMb) / $dt), (($load - $lastLoad) / $dt), [MemGen]::Errors, (Free-GB $dataRoot), $script:save.Count, (Save-Running), [MemSampler]::LastLine, (Status-Line))
                $lastOps = $ops; $lastMb = $mb; $lastLoad = $load; $lastT = Get-Date
            }
            if ((Free-GB $dataRoot) -lt 10 -or (Free-GB $loadDir) -lt 10) { $stopNote = 'stopped early: under 10 GB free'; Warn "$scn ($V): the churn was stopped early, under 10 GB free"; break }
        }
    } finally { [MemGen]::StopChurn() }
    [MemSampler]::SampleNow()
    $churnTime = ((Get-Date) - $t).TotalSeconds
    $ops = [double][MemGen]::Ops; $mb = [MemGen]::Bytes / 1MB; $errors = [MemGen]::Errors; $loadMb = [MemGen]::LoadBytes / 1MB
    $savesChurn = $script:save.Count; $longestChurn = [Math]::Max($script:save.Longest, (Save-Running))
    $saveAtEnd = Save-Running
    Remove-Item -LiteralPath $loadDir -Recurse -Force -ErrorAction SilentlyContinue
    Say ("disk load: {0:N0} MB written = {1:N0} MB/s; saves of index.bin seen during the churn: {2}, the longest about {3:N0} s{4}" -f $loadMb, ($loadMb / [Math]::Max(1.0, $churnTime)), $savesChurn, $longestChurn, $(if ($saveAtEnd -gt 0) { "; one was still running after $([int]$saveAtEnd) s when the churn stopped" } else { '' }))
    $kinds = "render {0:N0}, Unity Library {1:N0}, downloads {2:N0}, documents {3:N0}" -f [MemGen]::OpsRender, [MemGen]::OpsUnity, [MemGen]::OpsDownload, [MemGen]::OpsDocs
    Say ("churn done: {0:N0} file operations in {1:N0} s = {2:N0}/s ({3}); {4:N0} MB written = {5:N0} MB/s; {6} errors (last: {7})" -f $ops, $churnTime, ($ops / $churnTime), $kinds, $mb, ($mb / $churnTime), $errors, [MemGen]::LastError)
    if ($errors -gt 0) { Note "$scn ($V): the churn generator had $errors errors (files the program held open are normal), last: $([MemGen]::LastError)" }
    Shot "$V-$scn-after-churn"

    Set-Phase $V $scn 'after'
    [void](Wait-Sampled (Plan-Seconds 600 'F quiet time after the churn') "${scn}: ten minutes after the churn stopped ($V)")
    $statusAfter = Status-Line
    # Deleting the churn's files is one more burst of changes.
    Set-Phase $V $scn 'cleanup'
    if (-not (Remove-Tree $root 120)) { Note "$scn ($V): the churn's files were not all deleted within 2 min" }
    [void](Wait-Sampled (Plan-Seconds 60 'F after deleting the churn files') "${scn}: after deleting the churn's files ($V)")

    $ch = Get-Ticks $V $scn @('churn')
    $af = Get-Ticks $V $scn @('after')
    $all = Get-Ticks $V $scn @('pre', 'churn', 'after', 'cleanup')
    $p0 = Median (@(Last-Seconds $pre 30) | ForEach-Object { $_.Private })
    $pMax = Max-Of ($ch | ForEach-Object { $_.Private })
    $pEnd = Median (@(Last-Seconds $ch 15) | ForEach-Object { $_.Private })
    $pAfter = Median (@(Last-Seconds $af 30) | ForEach-Object { $_.Private })
    $growth = if ($ops -gt 0) { ($pEnd - $p0) / ($ops / 1e6) } else { [double]::NaN }
    $fit = Slope @($ch | ForEach-Object { $_.Ops / 1e6 }) @($ch | ForEach-Object { $_.Private })
    $rescans = Log-Count 'rescan [A-Za-z]:'
    $threadsBefore = Median (@(Last-Seconds $pre 30) | ForEach-Object { $_.Threads })
    $threadsMax = Max-Of (@($ch) + @($af) | ForEach-Object { $_.Threads })
    $saveThreadsMax = Max-Of (@($ch) + @($af) | ForEach-Object { $_.IndexSave })
    $named = @(Read-Samples | Where-Object { $_.version -eq $V -and $_.scenario -eq $scn -and $_.pid -ne '0' } | Select-Object -Last 1 | ForEach-Object { $_.thread_names }) -join ''
    $maxAll = Max-Of (@($ch) + @($af) | ForEach-Object { $_.Private })
    $ratioMax = if ($p0 -gt 0) { $maxAll / $p0 } else { [double]::NaN }
    $ratioAfter = if ($p0 -gt 0) { $pAfter / $p0 } else { [double]::NaN }
    $how = if ($Hook) { "saves delayed by $SaveDelayVar=$SaveDelayMs" } else { "$burners CPU burners, $loaders disk load threads" }
    # The council's criteria (set for the branch build; measured for every exe).
    $name = if ($Hook) { 'H' } else { 'F' }
    Add-Criterion "$name. Churn ($how)" $V 'private bytes stay within 2.2x the idle baseline of the same index (max during the churn and the 10 min after)' (-not [double]::IsNaN($ratioMax) -and $ratioMax -le 2.2) ("max {0:N0} MB = {1:N2}x of {2:N0} MB; {3:N2}x 10 min after" -f $maxAll, $ratioMax, $p0, $ratioAfter)
    Add-Criterion "$name. Churn ($how)" $V 'thread count flat (max at most 8 over the baseline; at most 2 index-save threads)' ($threadsMax -le $threadsBefore + 8 -and $saveThreadsMax -le 2) ("threads {0:N0} before, max {1:N0}; index-save threads max {2}" -f $threadsBefore, $threadsMax, $saveThreadsMax)
    $churnRows.Add([pscustomobject]@{
        Scenario = $name; How = $how; RatioMax = [Math]::Round($ratioMax, 2); RatioAfter = [Math]::Round($ratioAfter, 2)
        Version = $V; ChurnSeconds = [int]$churnTime; Ops = [int64]$ops; OpsPerSec = [int]($ops / [Math]::Max(1.0, $churnTime)); WrittenMB = [int64]$mb; LoadMB = [int64]$loadMb
        ThreadsMax = $threadsMax; IndexSaveThreadsMax = $saveThreadsMax; SavesSeen = $script:save.Count; LongestSaveS = [Math]::Round([Math]::Max($script:save.Longest, $longestChurn), 0)
        PrivateBefore = [Math]::Round($p0, 1); PrivateMaxChurn = [Math]::Round($pMax, 1); PrivateEndChurn = [Math]::Round($pEnd, 1); PrivateAfter = [Math]::Round($pAfter, 1)
        GrowthPerMillion = [Math]::Round($growth, 2); FitPerMillion = [Math]::Round($fit, 2); KeptMB = [Math]::Round($pAfter - $p0, 1)
        Rescans = $rescans; Errors = $errors; Kinds = $kinds; Leg = $Leg
    })
    $ratios = "against the idle baseline of the same index: max {0:N2}x, 10 min after {1:N2}x" -f $ratioMax, $ratioAfter
    $saves = "saves of index.bin seen: {0} during the churn (longest about {1:N0} s{2}), {3} in all; threads {4:N0} before, max {5:N0}; index-save threads max {6}" -f $savesChurn, $longestChurn, $(if ($saveAtEnd -gt 0) { ", one still running after $([int]$saveAtEnd) s at the end" } else { '' }), $script:save.Count, $threadsBefore, $threadsMax, $saveThreadsMax
    $text = "$how; $ratios; {0:N0} file operations in {1:N0} s ({2:N0}/s; {3}), {4:N0} MB written, disk load {14:N0} MB; private {5:N0} MB before, max {6:N0} during, {7:N0} at the end of the churn, {8:N0} MB 10 min after; growth {9:N1} MB per million operations (fit {10:N1}); {15}; whole-drive rescans in log.txt: {11}; status bar after the quiet time: {12} {13}" -f $ops, $churnTime, ($ops / [Math]::Max(1.0, $churnTime)), $kinds, $mb, $p0, $pMax, $pEnd, $pAfter, $growth, $fit, $rescans, $statusAfter, $stopNote, $loadMb, $saves
    if ($Hook) { Add-Row 62 'H. Churn with index saves delayed by the test hook: 15 min in the tray, then 10 min quiet' $V $all $af $text }
    else { Add-Row 60 'F. Churn with every core and the disk busy: 15 min in the tray, then 10 min quiet' $V $all $af $text }
    $facts = New-Object System.Collections.Generic.List[string]
    $facts.Add("Name index: $($s.Text); $($tree.SmallFiles) synthetic small files on disk.")
    $facts.Add("${scn}: $text.")
    $facts.Add("Threads by name at the last sample: $named")
    foreach ($l in $script:save.Lines) { $facts.Add("Save: $l") }
    foreach ($l in @(Log-Lines 'index|save|rescan|watch' 150)) { $facts.Add("log.txt: ``$l``") }
    $facts.Add("Exit: $(Exit-Probe $V $scn).")
    Finish-Session $V $scn "$scn. Churn ($how), $V" $facts.ToArray() $hash
}

# ---------------------------------------------------------------- O: watch-buffer overflow
# Whole-drive rescans the program finished, per root ("C:\" -> 3), from log.txt.
function Rescan-Counts {
    Sync-Log
    $h = @{}
    foreach ($m in [regex]::Matches($script:logAll, 'rescan ([A-Za-z]:\\): \d+ entries')) { $k = $m.Groups[1].Value; $h[$k] = 1 + [int]$h[$k] }
    return $h
}
# How many processes a timeline shows `Seconds` after second `From` (the last entry when it ends before).
function Count-At($Timeline, [double]$From, [double]$Seconds) {
    $all = @($Timeline)
    foreach ($e in $all) { if ($e.T -ge $From + $Seconds) { return @($e.Ids).Count } }
    if ($all.Count) { return @($all[-1].Ids).Count }
    return 0
}

function Scenario-O([string]$V) {
    $hash = File-Hash $exes[$V]
    $s = Start-Fresh $V 'O' @('--startup')
    Set-Phase $V 'O' 'pre'
    [void](Wait-Sampled (Plan-Seconds 90 'O idle baseline') "O: idle baseline ($V)")
    $pre = Get-Ticks $V 'O' @('pre')
    $p0 = Median (@(Last-Seconds $pre 30) | ForEach-Object { $_.Private })
    $root = Join-Path $dataRoot 'bursts'
    $files = [int][Math]::Max(2000, 150000 * [Math]::Min(1.0, $SizeScale))
    $bursts = [int][Math]::Max(2, [Math]::Round(5 * [Math]::Min(1.0, $TimeScale)))
    $start = Rescan-Counts
    $lines = New-Object System.Collections.Generic.List[string]
    $done = 0
    for ($b = 1; $b -le $bursts; $b++) {
        if ((Time-Up) -or (Time-Left) -lt 600) { Note "O ($V): stopped after $done of $bursts bursts, the time budget is nearly used up"; break }
        if (-not (Get-Process bahth -ErrorAction SilentlyContinue)) { throw "APP-EXITED: the program went away before burst $b" }
        if ((Free-GB $dataRoot) -lt 10) { Warn "O ($V): under 10 GB free; no more bursts"; break }
        Set-Phase $V 'O' 'burst'
        $t = Get-Date
        $ops = [MemGen]::Burst((Join-Path $root "burst-$b"), $files, $true)
        $secs = [Math]::Max(0.1, ((Get-Date) - $t).TotalSeconds)
        $done++
        Set-Phase $V 'O' 'gap'
        [void](Wait-Sampled (Plan-Seconds 90 'O gap after a burst') "O: after burst $b ($V)")
        $now = Rescan-Counts
        $line = "burst {0}: {1:N0} files created and deleted = {2:N0} file operations in {3:N1} s ({4:N0}/s); rescans finished so far: {5}; {6}" -f $b, $files, $ops, $secs, ($ops / $secs), $(if ($now.Count) { (@($now.Keys | Sort-Object | ForEach-Object { "$_ $($now[$_])" }) -join ', ') } else { 'none' }), [MemSampler]::LastLine
        Say $line
        $lines.Add($line)
    }
    [void](Remove-Tree $root 60)
    Set-Phase $V 'O' 'after'
    [void](Wait-Sampled (Plan-Seconds 300 'O quiet time after the bursts') "O: five minutes after the bursts ($V)")
    $end = Rescan-Counts
    $worst = 0; $perRoot = @()
    foreach ($k in @($end.Keys | Sort-Object)) { $n = [int]$end[$k] - [int]$start[$k]; $perRoot += "$k $n"; if ($n -gt $worst) { $worst = $n } }
    $all = Get-Ticks $V 'O' @('pre', 'burst', 'gap', 'after')
    $af = Get-Ticks $V 'O' @('after')
    $pMax = Max-Of ($all | ForEach-Object { $_.Private })
    $pAfter = Median (@(Last-Seconds $af 30) | ForEach-Object { $_.Private })
    $ratio = if ($p0 -gt 0) { $pAfter / $p0 } else { [double]::NaN }
    $name = 'O. Watch-buffer overflow bursts'
    Add-Criterion $name $V 'at most one whole-drive rescan per root per burst' ($done -gt 0 -and $worst -le $done) ("{0} bursts; rescans finished per root: {1}" -f $done, $(if ($perRoot.Count) { $perRoot -join ', ' } else { 'none' }))
    Add-Criterion $name $V 'memory back within 2.2x the idle baseline 5 min after the bursts' (-not [double]::IsNaN($ratio) -and $ratio -le 2.2) ("{0:N0} MB = {1:N2}x of {2:N0} MB; max {3:N0} MB" -f $pAfter, $ratio, $p0, $pMax)
    $text = "{0} bursts of {1:N0} files created and deleted; private {2:N0} MB idle, max {3:N0}, {4:N0} MB ({5:N2}x) 5 min after; whole-drive rescans finished per root: {6}; threads max {7:N0}" -f $done, $files, $p0, $pMax, $pAfter, $ratio, $(if ($perRoot.Count) { $perRoot -join ', ' } else { 'none' }), (Max-Of ($all | ForEach-Object { $_.Threads }))
    Add-Row 64 'O. Watch-buffer overflow: bursts of files created and deleted in seconds, then 5 min quiet' $V $all $af $text
    $facts = New-Object System.Collections.Generic.List[string]
    $facts.Add("Name index: $($s.Text).")
    $facts.Add("O: $text.")
    foreach ($l in $lines) { $facts.Add($l) }
    foreach ($l in @(Log-Lines 'rescan|watch' 60)) { $facts.Add("log.txt: ``$l``") }
    Finish-Session $V 'O' "O. Watch-buffer overflow, $V" $facts.ToArray() $hash
}

# ---------------------------------------------------------------- K: killed in the middle of a save
function Scenario-K([string]$V) {
    $hash = File-Hash $exes[$V]
    # With the branch build's hook a save lasts 5 s, so the kill lands inside it.
    $hook = ($V -eq 'branch' -and $SaveDelayMs -gt 0)
    if ($hook) { Set-Item -Path "Env:$SaveDelayVar" -Value '5000'; Say "hook: $SaveDelayVar=5000 for these starts" }
    try {
        $s = Start-Fresh $V 'K' @()
        $cycles = [int][Math]::Max(2, [Math]::Round(20 * [Math]::Min(1.0, $TimeScale)))
        $root = Join-Path $dataRoot 'kill'
        $tmp = Join-Path $data 'index.tmp'
        $loaded = 0; $rebuilt = 0; $unknown = 0; $noWindow = 0; $midSave = 0; $done = 0
        $crashesBefore = $crashes.Count
        $builds = Log-Count 'index build: \d+ entries'
        for ($i = 1; $i -le $cycles; $i++) {
            if ((Time-Up) -or (Time-Left) -lt 420) { Note "K ($V): stopped after $done of $cycles kills, the time budget is nearly used up"; break }
            Set-Phase $V 'K' 'change'
            # A change the program watches: it saves the index 30 s later.
            [void][MemGen]::Burst((Join-Path $root "c$i"), 400, $false)
            $sawSave = Wait-Until { Test-Path -LiteralPath $tmp } 50
            Start-Sleep -Milliseconds (Get-Random -Minimum 0 -Maximum $(if ($hook) { 4500 } else { 1200 }))
            $inSave = Test-Path -LiteralPath $tmp
            if ($inSave) { $midSave++ }
            [MemSampler]::SampleNow()
            Get-Process bahth -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
            [void](Wait-Until { -not (Get-Process bahth -ErrorAction SilentlyContinue) } 10)
            $done++
            Set-Phase $V 'K' 'restart'
            Start-App @()
            $main = Wait-Until { Main-Window } 40
            if (-not $main) { $noWindow++; Note "K ($V): no window after kill $i"; Check-Crash "K kill $i"; continue }
            $state = Wait-Until {
                $st = Status-Line
                if ($st -match 'Index loaded|تم تحميل الفهرس') { 'loaded' }
                elseif ($st -match 'Indexing files on all drives|files so far|جاري فهرسة الملفات|ملف حتى الآن') { 'rebuild' }
            } 25
            Check-Crash "K kill $i"
            if ($state -eq 'loaded') { $loaded++ }
            elseif ($state -eq 'rebuild') {
                $rebuilt++
                Shot "$V-K-rebuild-$i"
                # Let it finish, or every later kill would hit the rebuild instead of a save.
                [void](Wait-Sampled (Plan-Seconds 900 'K rebuild of the index' 240 600) "K: the index is rebuilt after kill $i ($V)" { (Log-Count 'index build: \d+ entries') -gt $builds })
                $builds = Log-Count 'index build: \d+ entries'
            } else { $unknown++; if ($unknown -le 3) { Note "K ($V): after kill $i the status bar says neither loaded nor indexing: '$(Status-Line)'" } }
            Say ("K kill {0}/{1}: index.tmp seen {2}, killed during a save {3}, next start: {4}" -f $i, $cycles, [bool]$sawSave, $inSave, $(if ($state) { $state } else { 'unknown' }))
        }
    } finally { if ($hook) { Remove-Item -Path "Env:$SaveDelayVar" -ErrorAction SilentlyContinue } }
    [void](Remove-Tree (Join-Path $dataRoot 'kill') 60)
    $newCrashes = $crashes.Count - $crashesBefore
    $laterBuilds = (Log-Count 'index build: \d+ entries') - 1
    $text = "{0} kills ({1} while index.tmp was there); next start loaded the index {2} times, rebuilt it {3} times, unclear {4}, no window {5}; crash reports {6}; index builds in log.txt after the first: {7}{8}" -f $done, $midSave, $loaded, $rebuilt, $unknown, $noWindow, $newCrashes, $laterBuilds, $(if ($hook) { "; saves delayed 5 s by $SaveDelayVar" } else { '' })
    Add-Criterion 'K. Killed in the middle of a save' $V 'index.bin loads on every next start (no rebuild, no crash report)' ($done -gt 0 -and $rebuilt -eq 0 -and $laterBuilds -le 0 -and $noWindow -eq 0 -and $newCrashes -eq 0) $text
    $all = Get-Ticks $V 'K' @('change', 'restart')
    Add-Row 66 'K. Killed in the middle of saving index.bin, again and again' $V $all $all $text
    Finish-Session $V 'K' "K. Killed mid-save, $V" @("Name index: $($s.Text).", "K: $text.", 'Two savers at once are not provoked from outside; only the kills are.') $hash
}

# ---------------------------------------------------------------- M, S, Q: handover, suspended copy, session end
# M: a copy in Downloads accepts the move to its permanent place while a
# save of the index is pending, and restarts from there with --moved.
function Do-Moved([string]$V) {
    Reset-Data
    Remove-Item -LiteralPath $instDir -Recurse -Force -ErrorAction SilentlyContinue
    Write-Settings @{ move_asked = '0' }
    $script:logCopy = Join-Path $logsDir "$V-M-log.txt"
    $dl = Join-Path $env:USERPROFILE 'Downloads'
    $dlExe = Join-Path $dl 'bahth.exe'
    Copy-Exe $exes[$V] $dlExe
    $lnk = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
    $offerMatch = 'permanent place|مكان دائم'
    $findOffer = { Find-Dialogs | Where-Object { "$($_.Title) $($_.Body)" -match $offerMatch } | Select-Object -First 1 }
    try {
        Set-Phase $V 'M' 'index'
        Say "starting $dlExe (a copy in Downloads)"
        $script:app = Start-Process -FilePath $dlExe -WorkingDirectory $dl -PassThru
        $offer = Wait-Until $findOffer 60
        if (-not $offer) { Shot "$V-M-no-offer"; Dump-State "M: no move offer ($V)"; throw 'M: no offer to move to a permanent place within 60 s' }
        Say "move offer: $($offer.Body)"
        Shot "$V-M-offer"
        # The offer stays open while the name index is built.
        $max = Plan-Seconds $script:indexMax "name index ($V, M)" 240 $script:indexMax
        $ok = Wait-Sampled $max "the name index of $V while the move offer is open" { Index-Ready } -AllowDialogs
        if (-not $ok) { Note "M ($V): the name index was not ready after $max s" }
        Start-Sleep -Seconds 5
        $oldId = @(Bahth-Ids)[0]
        $builds = Log-Count 'index build: \d+ entries'
        # A change the program watches: a save of the index is now pending
        # (written 30 s later, or when the program closes).
        [void][MemGen]::Burst((Join-Path $dataRoot 'moved-change'), 2000, $false)
        Start-Sleep -Seconds 5
        Set-Phase $V 'M' 'handover'
        $offer = & $findOffer
        if (-not $offer) { throw 'M: the move offer is gone before it was answered' }
        $yes = [MemUi]::GetDlgItem($offer.Handle, 6)
        if ($yes -ne [IntPtr]::Zero) { [void][MemUi]::PostMessage($yes, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero) } else { Focus-Window $offer.Handle; [MemUi]::Chord($VK.ENTER) }
        $tl = Observe (Plan-Seconds 90 'M handover' 240 75) 'M handover'
        $newAt = First-Seen $tl { $_ -ne $oldId }
        $goneAt = Gone-At $tl $oldId
        $count15 = if ($newAt -ge 0) { Count-At $tl $newAt 15 } else { -1 }
        Set-Phase $V 'M' 'after'
        [void](Wait-Sampled (Plan-Seconds 45 'M after the handover' 240 30) "M: after the handover ($V)" -AllowNoProcess)
        $rebuild = (Log-Count 'index build: \d+ entries') -gt $builds
        $status = Status-Line
        if ($status -match 'Indexing files on all drives|files so far') { $rebuild = $true }
        $relaunched = Log-Has 'start: relaunched from the permanent place'
        $ids = @(Bahth-Ids)
        $where = @(Get-Process bahth -ErrorAction SilentlyContinue | ForEach-Object { $_.Path }) -join ', '
        $text = "a new process appeared {0} after Yes, the old one (pid {1}) {2}; processes 15 s after the new one appeared: {3}; at the end {4} [{5}] running from {6}; relaunch logged: {7}; index rebuilt after the handover: {8}; status bar: {9}" -f $(if ($newAt -ge 0) { '{0:N0} s' -f $newAt } else { 'NEVER' }), $oldId, $(if ($goneAt -ge 0) { 'was gone after {0:N0} s' -f $goneAt } else { 'was STILL RUNNING when the watch ended' }), $count15, $ids.Count, ($ids -join ', '), $where, $relaunched, $rebuild, $status
        Add-Criterion 'M. Handover with a save pending (--moved)' $V 'exactly one bahth process 15 s after the handover, no index rebuild' ($newAt -ge 0 -and $count15 -eq 1 -and $ids.Count -eq 1 -and -not $rebuild) $text
        $all = Get-Ticks $V 'M' @('handover', 'after')
        Add-Row 68 'M. Handover: a copy in Downloads moves to its permanent place and restarts (--moved), a save pending' $V $all (Get-Ticks $V 'M' @('after')) $text
        Shot "$V-M-end"
        return "M: $text."
    } finally {
        Remove-Item -LiteralPath $dlExe -Force -ErrorAction SilentlyContinue
        [void](Remove-Tree (Join-Path $dataRoot 'moved-change') 30)
        # The Start-menu shortcut the move made.
        foreach ($f in @(Get-ChildItem -LiteralPath $lnk -Filter '*.lnk' -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^(InstaFind|بحث سريع)\.lnk$' })) { Remove-Item -LiteralPath $f.FullName -Force -ErrorAction SilentlyContinue }
    }
}

# S: the first copy is frozen (every thread suspended); a second start must not hang with it.
function Do-Suspended([string]$V) {
    Set-Phase $V 'S' 'suspended'
    $first = @(Bahth-Ids)
    if (-not $first.Count) { return 'S: the program is not running' }
    $id = $first[0]
    $path = (Get-Process -Id $id).Path
    if (-not $path) { $path = $instExe }
    $frozen = $false
    try { $frozen = [MemSampler]::Suspend($id, $true) } catch { Note "S ($V): suspending pid $id failed: $_" }
    if (-not $frozen) { Note "S ($V): pid $id could not be suspended; skipped"; return 'S: the first copy could not be suspended' }
    $exited = $false; $secs = 0.0; $during = @()
    try {
        $t = Get-Date
        $p2 = Start-Process -FilePath $path -WorkingDirectory (Split-Path $path -Parent) -PassThru
        $exited = $p2.WaitForExit(30000)
        $secs = ((Get-Date) - $t).TotalSeconds
        [MemSampler]::SampleNow()
        $during = @(Bahth-Ids)
        if (-not $exited) { Say "the second start (pid $($p2.Id)) is still running after 30 s; ending it"; try { $p2.Kill() } catch { } }
    } finally { [void][MemSampler]::Suspend($id, $false) }
    Start-Sleep -Seconds 3
    $text = "second start of {0} while pid {1} was suspended: {2}; processes at that time [{3}]; after resuming: [{4}]" -f $path, $id, $(if ($exited) { 'came back after {0:N1} s' -f $secs } else { 'STILL RUNNING after 30 s (ended by the test)' }), ($during -join ', '), ((Bahth-Ids) -join ', ')
    Add-Criterion 'S. Second start while the first copy is suspended' $V 'the second start comes back within 5 s, no second full instance' ($exited -and $secs -le 5) $text
    $ticks = Get-Ticks $V 'S' @('suspended')
    Add-Row 72 'S. Second start while the first copy is suspended' $V $ticks $ticks $text
    return "S: $text."
}

# Q: Windows ends the session: every top-level window is asked
# (WM_QUERYENDSESSION), then told (WM_ENDSESSION).
function Do-SessionEnd([string]$V) {
    Set-Phase $V 'Q' 'session-end'
    $ids = @(Bahth-Ids)
    if (-not $ids.Count) { return 'Q: the program is not running' }
    $wins = @(Bahth-Windows | Where-Object { $_.Visible -or $_.Class -eq 'NativeWindowsGuiWindow' })
    [MemSampler]::SampleNow()
    $t = Get-Date
    $refused = 0
    foreach ($w in $wins) { if ([MemUi]::Send($w.Handle, 0x0011, [IntPtr]::Zero, [IntPtr]::Zero) -eq [IntPtr]::Zero) { $refused++ } }
    foreach ($w in $wins) { [void][MemUi]::Send($w.Handle, 0x0016, [IntPtr]1, [IntPtr]::Zero) }
    $sent = ((Get-Date) - $t).TotalSeconds
    $gone = Wait-Until { -not (Get-Process bahth -ErrorAction SilentlyContinue) } 30
    $secs = ((Get-Date) - $t).TotalSeconds
    $text = "WM_QUERYENDSESSION and WM_ENDSESSION sent to {0} windows in {1:N1} s ({2} did not answer yes): the program {3}" -f $wins.Count, $sent, $refused, $(if ($gone) { 'exited after {0:N1} s' -f $secs } else { 'is STILL RUNNING after 30 s' })
    Add-Criterion 'Q. Windows session end' $V 'the process exits within 5 s' ([bool]$gone -and $secs -le 5) $text
    $ticks = Get-Ticks $V 'Q' @('session-end')
    Add-Row 74 'Q. Windows session end (WM_QUERYENDSESSION, WM_ENDSESSION)' $V $ticks $ticks $text
    return "Q: $text."
}

function Scenario-Handover([string]$V, [bool]$DoM, [bool]$DoS, [bool]$DoQ) {
    if ($DoM -and $V -eq '0.9.1') {
        Note 'M: 0.9.1 has no move to a permanent place; skipped for it'
        $DoM = $false
        if (-not ($DoS -or $DoQ)) { return }
    }
    $scn = if ($DoM) { 'M' } elseif ($DoS) { 'S' } else { 'Q' }
    $facts = New-Object System.Collections.Generic.List[string]
    if ($DoM) { $facts.Add((Do-Moved $V)) }
    if (-not (Get-Process bahth -ErrorAction SilentlyContinue)) {
        $s = Start-Fresh $V $scn @()
        $facts.Add("Name index: $($s.Text).")
    }
    if ($DoS) { $facts.Add((Do-Suspended $V)) }
    if ($DoQ) { $facts.Add((Do-SessionEnd $V)) }
    $names = @(); if ($DoM) { $names += 'M' }; if ($DoS) { $names += 'S' }; if ($DoQ) { $names += 'Q' }
    Finish-Session $V $scn "$($names -join ', '), $V" $facts.ToArray()
}

# ---------------------------------------------------------------- D: the update path
$updatePattern = 'Check for updates|التحقق من التحديثات'
$updateLog = ' update: (found|up to date|check failed|skipped)'

# The same data folder, another exe: `V` installed and started, window awaited.
function Start-Kept([string]$V, [string]$Scenario, [string]$Phase) {
    Stop-App
    Install-Exe $V
    Write-Settings
    Set-Phase 'update' $Scenario $Phase
    Start-App @()
    $main = Wait-Until { Main-Window } 40
    if (-not $main) { Dump-State "no main window ($V, $Phase)"; throw "no main window 40 s after starting $V ($Phase)" }
}

# D1: 0.9.1 checks for updates from its Options menu and installs what it finds.
function Scenario-D1 {
    $L = 'update'
    $hashOld = File-Hash $exes['0.9.1']
    $hashNew = File-Hash $exes['0.10.0']
    $s = Start-Fresh '0.9.1' 'D' @() @{} $L
    Set-Phase $L 'D' 'd1-before'
    [void](Wait-Sampled (Plan-Seconds 30 'D1 before the update') 'D1: 0.9.1 before the update')
    $oldId = @(Bahth-Ids)[0]
    $beforeLine = [MemSampler]::LastLine
    $facts = New-Object System.Collections.Generic.List[string]
    $facts.Add("Name index: $($s.Text).")
    $facts.Add("Before the update: $beforeLine")

    Set-Phase $L 'D' 'd1-update'
    $seenCheck = { (@(Find-Dialogs).Count -gt 0) -or (Log-Has $updateLog) }
    $trigger = 'Options > Check for updates, clicked with the mouse'
    [void](Invoke-OptionsItem $updatePattern)
    $seen = Wait-Until $seenCheck 45
    if (-not $seen -and (Post-OptionsItem $updatePattern)) {
        $trigger = 'Options > Check for updates, sent as WM_MENUCOMMAND (the click had no effect)'
        $seen = Wait-Until $seenCheck 45
    }
    if (-not $seen) {
        Note 'D1: the check could not be started from the Options menu; 0.9.1 restarted with update_check=0 for its automatic check'
        Stop-App
        Write-Settings @{ update_check = '0' }
        Start-App @()
        [void](Wait-Until { Main-Window } 40)
        $oldId = @(Bahth-Ids)[0]
        $trigger = 'the automatic check 10 s after a start (update_check=0)'
        $seen = Wait-Until $seenCheck 90
    }
    Say "D1 trigger: $trigger; something happened: $([bool]$seen)"
    $dlg = Wait-Until { Find-Dialogs | Select-Object -First 1 } 30
    $result = ''; $timing = ''
    if ($dlg) { Shot 'D1-dialog'; Say "D1 dialog '$($dlg.Title)': $($dlg.Body)" }
    if ($dlg -and $dlg.Body -match 'new version of InstaFind is available|يتوفر إصدار جديد') {
        $offer = One-Line $dlg.Body
        $facts.Add("The offer: $($offer.Substring(0, [Math]::Min(160, $offer.Length)))…")
        # A change the program watches, so a save of the index is pending
        # when the old copy hands over.
        $builds = Log-Count 'index build: \d+ entries'
        [void][MemGen]::Burst((Join-Path $dataRoot 'update-change'), 2000, $false)
        Start-Sleep -Seconds 3
        $yes = [MemUi]::GetDlgItem($dlg.Handle, 6)
        if ($yes -ne [IntPtr]::Zero) { [void][MemUi]::PostMessage($yes, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero) } else { Focus-Window $dlg.Handle; [MemUi]::Chord($VK.ENTER) }
        Start-Sleep -Seconds 2
        # Until the new copy says it started, the install failed, or a message box came up.
        $tl = Observe (Plan-Seconds 240 'D1 download and restart' 150 120) 'D1 update' { (Log-Has 'start: relaunched after updating|update: install failed') -or (@(Find-Dialogs).Count -gt 0) }
        $tl2 = Observe 20 'D1 just after'
        $whole = @($tl) + @($tl2 | ForEach-Object { [pscustomobject]@{ T = $_.T + $tl[-1].T + 1; Ids = $_.Ids } })
        $newAt = First-Seen $whole { $_ -ne $oldId }
        $goneAt = Gone-At $whole $oldId
        $timing = "a new process appeared {0} after Yes; the old process (pid {1}) {2}" -f $(if ($newAt -ge 0) { '{0:N0} s' -f $newAt } else { 'NEVER' }), $oldId, $(if ($goneAt -ge 0) { 'was gone {0:N0} s after Yes' -f $goneAt } else { 'was STILL RUNNING when the watch ended' })
        $err = @(Find-Dialogs) | Select-Object -First 1
        if ($err) {
            $result = "the update did not finish: $(One-Line $err.Body)"
            Shot 'D1-error'
            [void](Answer-Dialog $err @(1, 2, 7))
        } elseif ($script:logAll -match 'start: relaunched after updating to (\S+)') {
            $result = "0.9.1 updated itself to $($Matches[1])"
        } elseif ($script:logAll -match 'update: install failed: (.*)') {
            $result = "the install failed: $($Matches[1])"
        } else {
            $result = 'the update did not finish within the time allowed'
        }
        Set-Phase $L 'D' 'd1-after'
        [void](Wait-Sampled (Plan-Seconds 180 'D1 after the update' 150 45) 'D1: three minutes after the update' -AllowNoProcess)
        $count15 = if ($newAt -ge 0) { Count-At $whole $newAt 15 } else { -1 }
        $rebuild = (Log-Count 'index build: \d+ entries') -gt $builds
        Add-Criterion 'D1. Handover of the self-update with a save pending (--updated)' '0.9.1 -> latest' 'exactly one bahth process 15 s after the handover, no index rebuild' ($newAt -ge 0 -and $count15 -eq 1 -and -not $rebuild) ("{0}; processes 15 s after the new one appeared: {1}; index rebuilt afterwards: {2}" -f $timing, $count15, $rebuild)
        [void](Remove-Tree (Join-Path $dataRoot 'update-change') 30)
    } elseif ($dlg) {
        $result = "no update was offered: $(One-Line $dlg.Body)"
        [void](Answer-Dialog $dlg @(1, 2, 7))
        Set-Phase $L 'D' 'd1-after'
        [void](Wait-Sampled (Plan-Seconds 30 'D1 after') 'D1: after the dialog' -AllowNoProcess)
    } else {
        $lines = @($script:logAll -split "`r?`n" | Where-Object { $_ -match ' update: ' }) -join ' / '
        $result = "no message box appeared; update lines in log.txt: $(if ($lines) { $lines } else { '(none)' })"
        Set-Phase $L 'D' 'd1-after'
        [void](Wait-Sampled (Plan-Seconds 30 'D1 after') 'D1: after the check' -AllowNoProcess)
    }
    Shot 'D1-end'
    $hashNow = if (Test-Path -LiteralPath $instExe) { File-Hash $instExe } else { '' }
    $exeNow = if ($hashNow -eq $hashNew) { 'bahth.exe is now the released 0.10.0' } elseif ($hashNow -eq $hashOld) { 'bahth.exe is still 0.9.1' } elseif ($hashNow) { "bahth.exe is another build (SHA-256 $hashNow): a release newer than 0.10.0?" } else { 'bahth.exe is MISSING' }
    $leftover = @(Get-ChildItem -LiteralPath $instDir -ErrorAction SilentlyContinue | ForEach-Object { "$($_.Name) ($([int]($_.Length / 1KB)) KB)" }) -join ', '
    $ids = @(Bahth-Ids)
    if (-not $ids.Count) { Warn 'D1: no bahth process is running three minutes after the update' }
    $text = "{0}. Started by {1}. {2}. At the end: {3} process(es) [{4}]; {5}; folder: {6}" -f $result, $trigger, $timing, $ids.Count, ($ids -join ', '), $exeNow, $leftover
    $all = Get-Ticks $L 'D' @('d1-update', 'd1-after')
    $end = Get-Ticks $L 'D' @('d1-after')
    Add-Row 40 'D1. Self-update: 0.9.1 checks from Options and installs the latest release' '0.9.1 -> latest' $all $end $text
    $facts.Add("D1: $text.")
    Finish-Session $L 'D' 'D1. Self-update of 0.9.1' $facts.ToArray()
}

# Starts the other version beside a running one and watches for 40 s.
function Second-Start([string]$Running, [string]$Other, [string]$Phase, [int]$Order, [string]$Title) {
    $L = 'update'
    Set-Phase $L 'D' $Phase
    $first = @(Bahth-Ids)
    Copy-Exe $exes[$Other] $sideExe
    [MemSampler]::SampleNow()
    $firstLine = [MemSampler]::LastLine
    $p2 = Start-Process -FilePath $sideExe -WorkingDirectory $sideDir -PassThru
    Say "${Phase}: started $Other as pid $($p2.Id) while $Running runs as [$($first -join ', ')]"
    $tl = Observe (Plan-Seconds 40 $Phase 150 25) $Phase
    $gone = Gone-At $tl $p2.Id
    $ids = @(Bahth-Ids)
    $firstAlive = @($first | Where-Object { $ids -contains $_ }).Count
    $text = "{0} was started while {1} was running (pid {2}): the second process {3}; the first one is {4}; at the end {5} process(es) [{6}]. First before: {7}" -f $Other, $Running, ($first -join ', '), $(if ($gone -ge 0) { 'exited after {0:N0} s' -f $gone } else { 'is STILL RUNNING after 40 s' }), $(if ($firstAlive) { 'still running' } else { 'GONE' }), $ids.Count, ($ids -join ', '), $firstLine
    Shot $Phase
    $ticks = Get-Ticks $L 'D' @($Phase)
    Add-Row $Order $Title "$Running, then $Other" $ticks $ticks $text
    foreach ($l in @(Process-List)) { Say "  $l" }
    return $text
}

# D3b: 0.10.0 runs, 0.9.1 is started.
function Scenario-D3b {
    Start-Kept '0.10.0' 'D' 'd3b-start'
    [void](Wait-Sampled (Plan-Seconds 45 'D3b start' 150 20) 'D3b: 0.10.0 running')
    $text = Second-Start '0.10.0' '0.9.1' 'd3b' 42 'D3b. 0.9.1 started while 0.10.0 runs'
    Finish-Session 'update' 'D' 'D3b. 0.9.1 started while 0.10.0 runs' @($text)
}

# D3a: 0.9.1 runs, 0.10.0 is started. Then D2: the updater's own steps are
# replayed by hand while that 0.9.1 stays alive.
function Scenario-D3aD2 {
    $L = 'update'
    Start-Kept '0.9.1' 'D' 'd3a-start'
    [void](Wait-Sampled (Plan-Seconds 60 'D3a start' 150 20) 'D3a: 0.9.1 running')
    $facts = New-Object System.Collections.Generic.List[string]
    $facts.Add((Second-Start '0.9.1' '0.10.0' 'd3a' 41 'D3a. 0.10.0 started while 0.9.1 runs'))

    # What update::download_and_install and restart_new_version do: the
    # running exe is renamed to bahth.exe.old, the new one takes its name
    # and is started with --updated. Here the old copy does NOT exit (as
    # when it is slow to close): the new one waits for it for up to a minute.
    Set-Phase $L 'D' 'd2'
    $first = @(Bahth-Ids)
    if (-not $first.Count) { throw 'D2: 0.9.1 is not running any more' }
    Remove-Item -LiteralPath ($instExe + '.old') -Force -ErrorAction SilentlyContinue
    Rename-Item -LiteralPath $instExe -NewName 'bahth.exe.old'
    Copy-Item -LiteralPath $exes['0.10.0'] -Destination $instExe -Force
    $p2 = Start-Process -FilePath $instExe -ArgumentList '--updated' -WorkingDirectory $instDir -PassThru
    Say "D2: started the swapped-in 0.10.0 with --updated as pid $($p2.Id) while 0.9.1 runs as [$($first -join ', ')]"
    $tl = Observe (Plan-Seconds 110 'D2' 150 80) 'D2'
    $gone = Gone-At $tl $p2.Id
    $ids = @(Bahth-Ids)
    $firstAlive = @($first | Where-Object { $ids -contains $_ }).Count
    $both = 0.0
    foreach ($e in $tl) { if (@($e.Ids).Count -ge 2) { $both = [double]$e.T } }
    $text = "the updater's swap replayed while 0.9.1 (pid {0}) stayed alive: the new copy started with --updated {1}; two processes were alive together for about {2:N0} s; the old copy is {3}; at the end {4} process(es) [{5}]: {6}" -f ($first -join ', '), $(if ($gone -ge 0) { 'exited after {0:N0} s' -f $gone } else { 'is STILL RUNNING after 110 s' }), $both, $(if ($firstAlive) { 'still running (the old version)' } else { 'gone' }), $ids.Count, ($ids -join ', '), ((Process-List) -join ' // ')
    Shot 'd2'
    $ticks = Get-Ticks $L 'D' @('d2')
    Add-Row 43 'D2. The update swap replayed: new copy started with --updated while the old one stays' '0.9.1, then 0.10.0 --updated' $ticks $ticks $text
    $facts.Add("D2: $text.")
    Finish-Session $L 'D' 'D3a and D2. 0.10.0 started beside a running 0.9.1' $facts.ToArray()
}

# ---------------------------------------------------------------- G: the new look
function Scenario-G([string]$V) {
    $s = Start-Fresh $V 'G' @() @{ new_ui = '1' }
    Set-Phase $V 'G' 'idle'
    [void](Wait-Sampled (Plan-Seconds 180 'G idle') 'G: the new look, three minutes after the name index')
    $webui = @($script:logAll -split "`r?`n" | Where-Object { $_ -match 'webui' } | ForEach-Object { $_ -replace '^\d+ ', '' }) -join ' / '
    $classic = [bool](Classic-Window)
    $wv = @(Get-Process msedgewebview2 -ErrorAction SilentlyContinue).Count
    Shot 'G-end'
    $text = "{0}; {1} msedgewebview2 processes; classic window on screen: {2}; log: {3}; new_ui in settings.txt now: {4}" -f $s.Text, $wv, $classic, $webui, $(if ((Test-Path -LiteralPath $settingsFile) -and ([MemSampler]::ReadShared($settingsFile) -match '(?m)^new_ui=(\d)')) { $Matches[1] } else { '?' })
    if ($wv -eq 0) { Note 'G: no msedgewebview2 process: the new look did not start (see the log lines in the report)' }
    $all = Get-Ticks $V 'G' @('index', 'idle')
    $end = Get-Ticks $V 'G' @('idle')
    Add-Row 70 'G. New look (WebView2 window), 3 min after the name index' $V $all $end $text
    Finish-Session $V 'G' "G. New look, $V" @("G: $text.")
}

# ---------------------------------------------------------------- run
$scenarioList = @($Scenarios.ToUpperInvariant() -split '[,; ]+' | Where-Object { $_ })
$exes = @{}
if ($Exe091) { $exes['0.9.1'] = (Resolve-Path $Exe091).Path }
if ($Exe0100) { $exes['0.10.0'] = (Resolve-Path $Exe0100).Path }
if ($ExeBranch) { $exes['branch'] = (Resolve-Path $ExeBranch).Path }
$versionList = @($Versions -split '[,; ]+' | Where-Object { $_ })
if (-not $versionList.Count) { $versionList = @($exes.Keys | Sort-Object { Version-Key $_ }) }
if (-not $versionList.Count) { throw 'no exe: pass -Exe091, -Exe0100 and/or -ExeBranch' }
foreach ($v in $versionList) { if (-not $exes.ContainsKey($v)) { throw "no exe for version ${v}: pass -Exe091, -Exe0100 or -ExeBranch (versions: 0.9.1, 0.10.0, branch)" } }
if (($scenarioList -contains 'D') -and -not ($exes.ContainsKey('0.9.1') -and $exes.ContainsKey('0.10.0'))) { throw 'scenario D needs both -Exe091 and -Exe0100' }
# The longest wait for the first name index (a runner's disk is sometimes throttled).
$script:indexMax = 1500

if (-not $DataRoot) {
    $best = [IO.DriveInfo]::GetDrives() | Where-Object { $_.DriveType -eq 'Fixed' -and $_.IsReady } | Sort-Object AvailableFreeSpace -Descending | Select-Object -First 1
    $sys = [IO.Path]::GetPathRoot($env:USERPROFILE)
    $DataRoot = if ($best -and $best.Name -ne $sys) { Join-Path $best.Name 'InstaFind-memtest' } else { Join-Path $env:USERPROFILE 'Documents\InstaFind-memtest' }
}
$dataRoot = $DataRoot
New-Item -ItemType Directory -Force $dataRoot | Out-Null

$machine = New-Object System.Collections.Generic.List[string]
try {
    $os = Get-CimInstance Win32_OperatingSystem
    $cs = Get-CimInstance Win32_ComputerSystem
    $machine.Add(("{0} {1}, {2} logical processors, {3:N0} GB RAM, PowerShell {4}, screen {5}x{6}" -f $os.Caption, $os.Version, $cs.NumberOfLogicalProcessors, ($cs.TotalPhysicalMemory / 1GB), $PSVersionTable.PSVersion, [Windows.Forms.SystemInformation]::VirtualScreen.Width, [Windows.Forms.SystemInformation]::VirtualScreen.Height))
} catch { $machine.Add("machine: $_") }
foreach ($d in @([IO.DriveInfo]::GetDrives() | Where-Object { $_.IsReady })) { $machine.Add(("drive {0} {1} {2}: {3:N0} GB free of {4:N0} GB" -f $d.Name, $d.DriveType, $d.DriveFormat, ($d.AvailableFreeSpace / 1GB), ($d.TotalSize / 1GB))) }
$machine.Add("synthetic files go to $dataRoot; run '$Leg': versions $($versionList -join ', '), scenarios $($scenarioList -join ', '), time scale $TimeScale, size scale $SizeScale, budget $BudgetMinutes min")
foreach ($v in @($exes.Keys | Sort-Object { Version-Key $_ })) {
    $said = ''
    try {
        $o = Join-Path $Out "version-$v.txt"
        $p = Start-Process -FilePath $exes[$v] -ArgumentList '--version' -NoNewWindow -PassThru -RedirectStandardOutput $o -RedirectStandardError (Join-Path $Out "version-$v-err.txt")
        if (-not $p.WaitForExit(20000)) { $p.Kill(); $said = '(--version did not return in 20 s)' } else { $said = One-Line ([IO.File]::ReadAllText($o, [Text.Encoding]::UTF8)) }
    } catch { $said = "(--version failed: $_)" }
    $machine.Add(("exe {0}: {1}, {2:N0} bytes, SHA-256 {3}, says: {4}" -f $v, $exes[$v], (Get-Item -LiteralPath $exes[$v]).Length, (File-Hash $exes[$v]), $said))
}
foreach ($l in $machine) { Say $l }
Get-Process bahth* -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

[MemSampler]::Version = 'none'; [MemSampler]::Scenario = 'setup'; [MemSampler]::Phase = 'setup'
[MemSampler]::Start($samplesFile, 5000)
Say "sampler started: $samplesFile"
if ($BudgetMinutes -gt 0) {
    # Six minutes after the budget: enough for the clean-up and the report.
    [MemSampler]::StartWatchdog([int](Time-Left), 360)
    Say "time budget: $BudgetMinutes min; the watchdog ends a hung run 6 min after that"
}
if ((Free-GB $dataRoot) -lt 12) { Warn ("only {0:N0} GB free on {1}: the synthetic data is cut to the minimum" -f (Free-GB $dataRoot), $dataRoot) }

try {
    if ($scenarioList -contains 'A') { foreach ($v in $versionList) { Run-Scenario "A $v" { Scenario-A $v } } }
    $doB = $scenarioList -contains 'B'; $doC = $scenarioList -contains 'C'; $doE = $scenarioList -contains 'E'
    if ($doB -or $doC -or $doE) { foreach ($v in $versionList) { Run-Scenario "B-C-E $v" { Scenario-Static $v $doB $doC $doE } } }
    if ($scenarioList -contains 'F') { foreach ($v in $versionList) { Run-Scenario "F $v" { Scenario-F $v $false } } }
    if ($scenarioList -contains 'H') {
        foreach ($v in $versionList) {
            if ($v -eq 'branch') { Run-Scenario "H $v" { Scenario-F $v $true } }
            else { Note "H was skipped for ${v}: only the branch build has the save-delay hook ($SaveDelayVar)" }
        }
    }
    if ($scenarioList -contains 'O') { foreach ($v in $versionList) { Run-Scenario "O $v" { Scenario-O $v } } }
    if ($scenarioList -contains 'K') { foreach ($v in $versionList) { Run-Scenario "K $v" { Scenario-K $v } } }
    $doM = $scenarioList -contains 'M'; $doS = $scenarioList -contains 'S'; $doQ = $scenarioList -contains 'Q'
    if ($doM -or $doS -or $doQ) { foreach ($v in $versionList) { Run-Scenario "M-S-Q $v" { Scenario-Handover $v $doM $doS $doQ } } }
    if ($scenarioList -contains 'D') {
        Run-Scenario 'D1' { Scenario-D1 }
        Run-Scenario 'D3b' { Scenario-D3b }
        Run-Scenario 'D3a-D2' { Scenario-D3aD2 }
    }
    # (0.9.1 has no new look.)
    if ($scenarioList -contains 'G') { foreach ($v in $versionList) { if ($v -ne '0.9.1') { Run-Scenario "G $v" { Scenario-G $v } } } }
} catch {
    Problem "the run stopped: $(One-Line "$_") (line $($_.InvocationInfo.ScriptLineNumber))"
} finally {
    [MemSampler]::Version = 'none'; [MemSampler]::Scenario = 'end'; [MemSampler]::Phase = 'end'
    try { Stop-App } catch { }
    try { [MemGen]::StopChurn() } catch { }
    try { [MemSampler]::Stop() } catch { }
    if ([MemSampler]::LastError) { Note "sampler: last error: $([MemSampler]::LastError)" }
    if (-not $KeepData) {
        Say "removing $dataRoot"
        if (-not (Remove-Tree $dataRoot 150)) { Note "the synthetic files in $dataRoot were not all deleted within 150 s" }
    }
    Remove-Item -LiteralPath (Join-Path $env:LOCALAPPDATA 'InstaFind-memtest-load') -Recurse -Force -ErrorAction SilentlyContinue
    foreach ($d in @($instDir, $sideDir)) { Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue }

    # ------------------------------------------------------------ report
    # Every bahth process the sampler saw: the "second process" question.
    $seen = New-Object System.Collections.Generic.List[string]
    try {
        foreach ($g in @(Read-Samples | Where-Object { $_.pid -ne '0' } | Group-Object { "$($_.pid) $($_.start_time)" })) {
            $a = $g.Group[0]; $z = $g.Group[-1]
            $peak = Max-Of ($g.Group | ForEach-Object { To-Num $_.private_mb })
            $seen.Add(("| {0} | {1} | {2} | {3} / {4} | {5:N0} s | {6:N0} | ``{7}`` | ``{8}`` |" -f $a.pid, $a.parent_pid, $a.start_time, $a.scenario, $a.version, ((To-Num $z.elapsed_s) - (To-Num $a.elapsed_s)), $peak, ($z.exe -replace '\|', '/'), ($a.cmdline -replace '\|', '/')))
        }
    } catch { Note "process table: $_" }

    $title = if ($Leg) { "InstaFind memory test, run ``$Leg``" } else { 'InstaFind memory test' }
    $md = New-Object Text.StringBuilder
    [void]$md.AppendLine("## $title")
    [void]$md.AppendLine('')
    foreach ($l in $machine) { [void]$md.AppendLine("- $l") }
    [void]$md.AppendLine(("- Took {0:N0} min." -f ((Get-Date) - $script:t0).TotalMinutes))
    [void]$md.AppendLine('')
    if ($rows.Count) {
        $compare = Build-Compare $rows
        if ($compare) { [void]$md.AppendLine($compare) }
        [void]$md.AppendLine((Build-Table $rows))
        [void]$md.AppendLine($tableLegend)
    } else { [void]$md.AppendLine('No results.') }
    if ($criteria.Count) {
        [void]$md.AppendLine('### Pass criteria (set for the branch build; the released exes are measured the same way)')
        [void]$md.AppendLine('')
        [void]$md.AppendLine((Build-Criteria $criteria))
    }
    if ($churnRows.Count) {
        [void]$md.AppendLine('### F, H. Churn: private bytes against the idle baseline and the file operations')
        [void]$md.AppendLine('')
        [void]$md.AppendLine((Build-ChurnTable $churnRows))
    }
    foreach ($kind in @(@($warnings, 'Warnings'), @($crashes, 'Crashes'), @($problems, 'Harness problems'), @($notes, 'Notes'))) {
        if ($kind[0].Count) {
            [void]$md.AppendLine("### $($kind[1])")
            [void]$md.AppendLine('')
            foreach ($l in $kind[0]) { [void]$md.AppendLine("- $l") }
            [void]$md.AppendLine('')
        }
    }
    $dm = New-Object Text.StringBuilder
    [void]$dm.AppendLine($details.ToString())
    if ($seen.Count) {
        [void]$dm.AppendLine('### Every bahth process seen')
        [void]$dm.AppendLine('')
        [void]$dm.AppendLine('| PID | Parent | Started (UTC) | First seen in | Seen for | Peak private MB | Exe (last seen) | Command line |')
        [void]$dm.AppendLine('|---|---|---|---|---|---|---|---|')
        foreach ($l in $seen) { [void]$dm.AppendLine($l) }
        [void]$dm.AppendLine('')
    }
    $headLength = $md.Length
    [void]$md.AppendLine('### Details')
    [void]$md.AppendLine('')
    [void]$md.AppendLine($dm.ToString())
    [void]$md.AppendLine('samples.csv has one line per bahth process every 5 s (PID, parent, start time, command line, memory, handles, threads); logs\ has log.txt of every run; shots\ has the screenshots.')

    $rows | Export-Csv -LiteralPath (Join-Path $Out 'summary.csv') -NoTypeInformation -Encoding UTF8
    if ($churnRows.Count) { $churnRows | Export-Csv -LiteralPath (Join-Path $Out 'churn.csv') -NoTypeInformation -Encoding UTF8 }
    if ($criteria.Count) { $criteria | Export-Csv -LiteralPath (Join-Path $Out 'criteria.csv') -NoTypeInformation -Encoding UTF8 }
    [IO.File]::WriteAllLines((Join-Path $Out 'warnings.txt'), $warnings.ToArray(), $utf8)
    [IO.File]::WriteAllLines((Join-Path $Out 'crashes.txt'), $crashes.ToArray(), $utf8)
    [IO.File]::WriteAllLines((Join-Path $Out 'problems.txt'), $problems.ToArray(), $utf8)
    [IO.File]::WriteAllLines((Join-Path $Out 'notes.txt'), $notes.ToArray(), $utf8)
    [IO.File]::WriteAllText((Join-Path $Out 'details.md'), $dm.ToString(), $utf8)
    [IO.File]::WriteAllText((Join-Path $Out 'report.md'), $md.ToString(), $utf8)
    if ($env:GITHUB_STEP_SUMMARY) {
        $text = $md.ToString()
        if ($text.Length -gt 900000) { $text = $text.Substring(0, 900000) + "`n`n(cut: the full report is in the artifact)`n" }
        [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $text, $utf8)
    }
    # The log gets the tables; the details are in the summary and the artifact.
    Write-Host $md.ToString(0, $headLength)
}
Say ("{0} result rows, {1} criteria ({2} failed by the branch build), {3} warnings, {4} notes, {5} harness problems, {6} crashes" -f $rows.Count, $criteria.Count, $script:criteriaFailed, $warnings.Count, $notes.Count, $problems.Count, $crashes.Count)
if ($problems.Count -or $crashes.Count) { exit 1 }
if ($FailOnCriteria -and $script:criteriaFailed -gt 0) { Write-Host "::error::mem-test: the branch build fails $($script:criteriaFailed) of the council's criteria"; exit 1 }
Say 'Memory test finished.'
exit 0
