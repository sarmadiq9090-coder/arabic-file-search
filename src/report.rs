//! Options ▸ «الإبلاغ عن مشكلة…» / "Report a problem…": the text the user
//! can share when something goes wrong (memory, slowness, a crash), built
//! and filtered here, free of Win32 so it is tested on every platform. The
//! window (`ui::report`) shows exactly this text before anything happens;
//! nothing is ever sent by the program itself.
//!
//! What it holds: the version, Windows, how the copy was installed, the
//! look, the size of the index and the content index, the data files'
//! sizes, the memory used now and at its peak, the uptime, the settings that
//! matter (no paths), and the last lines of `log.txt`.
//!
//! Privacy: every line (and the user's own words) goes through
//! [`crate::feedback::anonymize_with`] and then [`scrub`], which replaces
//! with «…» anything that looks like a path, a file name, an e-mail address,
//! an IP address or a web address. A bare drive (`C:\`) is kept: the 1.0
//! memory, save and handover lines name drives and numbers only.

use std::fmt::Write as _;

/// What a removed piece is replaced with.
pub const GONE: &str = "«…»";

/// How many lines of `log.txt` the report ends with.
pub const LOG_LINES: usize = 80;

/// The longest log line kept (in characters); longer ones are cut.
const MAX_LINE: usize = 300;

/// The most the user's own words may hold (in characters).
pub const MAX_WHAT: usize = 500;

/// The facts the report states, gathered by the window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub version: String,
    /// `Windows 11 24H2 (build 26100.2314)`.
    pub os: String,
    /// `x86_64`, `x86` or `aarch64`.
    pub arch: String,
    /// `ar` or `en`.
    pub lang: String,
    /// How this copy was installed: `Microsoft Store`, `winget`, `Scoop`,
    /// `Chocolatey` or `portable`.
    pub build: String,
    /// The new look is the window in use.
    pub new_look: bool,
    /// The new look is chosen in the settings (it may not be running).
    pub new_look_chosen: bool,
    /// Files and folders in the index.
    pub entries: usize,
    /// Documents in the content index; `None` while it is not open.
    pub documents: Option<usize>,
    /// Size of `index.bin`.
    pub index_bytes: Option<u64>,
    /// Size of the content store (the `content` folder).
    pub content_bytes: Option<u64>,
    /// Private bytes now and at their peak.
    pub private_bytes: Option<u64>,
    pub peak_bytes: Option<u64>,
    /// Seconds since the program started.
    pub uptime_secs: u64,
    pub auto_index: bool,
    pub content_index: bool,
    /// Phone search: switched on, and running now.
    pub phone_on: bool,
    pub phone_running: bool,
}

/// The report: the user's words (filtered) and the rest, kept apart because
/// the issue form has a field for each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub what: String,
    pub details: String,
}

impl Report {
    /// Exactly what is shared, whichever button is pressed.
    pub fn text(&self) -> String {
        let what = if self.what.is_empty() { "-" } else { &self.what };
        format!("What happened:\n{what}\n\n{}", self.details)
    }
}

/// Builds the report from `facts`, the user's words `what` and the whole of
/// `log` (the text of `log.txt`), hiding each of `names` (the account and
/// computer names) as well as paths and the like.
pub fn build(facts: &Facts, what: &str, log: &str, names: &[&str]) -> Report {
    let what: String = what.chars().filter(|c| !c.is_control() || *c == '\n').take(MAX_WHAT).collect();
    let what = clean(what.trim(), names).replace("\r\n", "\n");
    let mut d = String::new();
    let _ = writeln!(d, "InstaFind {} on {} ({}), interface {}", facts.version, facts.os, facts.arch, facts.lang);
    let _ = writeln!(d, "Installed: {}", facts.build);
    let look = match (facts.new_look, facts.new_look_chosen) {
        (true, _) => "new",
        (false, true) => "classic (the new look is chosen but not running)",
        (false, false) => "classic",
    };
    let _ = writeln!(d, "Look: {look}");
    let docs = facts.documents.map_or("not open".to_string(), |n| n.to_string());
    let _ = writeln!(d, "Index: {} files and folders; content index: {docs} documents", facts.entries);
    let _ = writeln!(d, "Data files: index.bin {}; content store {}", size(facts.index_bytes), size(facts.content_bytes));
    let _ = writeln!(d, "Memory: private {} now, peak {}", size(facts.private_bytes), size(facts.peak_bytes));
    let _ = writeln!(d, "Running for: {}", uptime(facts.uptime_secs));
    let on = |b: bool| if b { "on" } else { "off" };
    let phone = match (facts.phone_on, facts.phone_running) {
        (true, true) => "on (running)",
        (true, false) => "on (not running now)",
        (false, _) => "off",
    };
    let _ = writeln!(
        d,
        "Settings: update the index automatically {}, content index {}, phone search {phone}",
        on(facts.auto_index),
        on(facts.content_index)
    );
    let tail = log_tail(log, LOG_LINES, names);
    let _ = write!(d, "\nLast {} lines of log.txt (file names and paths removed):\n", tail.lines().count());
    d.push_str(if tail.is_empty() { "(empty)" } else { &tail });
    Report { what, details: d }
}

/// `1234 MB`, or `unknown`.
fn size(bytes: Option<u64>) -> String {
    bytes.map_or("unknown".to_string(), |b| format!("{} MB", b.div_ceil(1024 * 1024)))
}

/// `3 h 12 min`, `12 min`, `40 s`.
fn uptime(secs: u64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    match (h, m) {
        (0, 0) => format!("{secs} s"),
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// The last `n` lines of `log`, each anonymised, filtered and cut to a sane
/// length.
pub fn log_tail(log: &str, n: usize, names: &[&str]) -> String {
    let lines: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
    let from = lines.len().saturating_sub(n);
    let mut out = String::new();
    for line in &lines[from..] {
        let line: String = line.chars().filter(|c| !c.is_control() || *c == '\t').collect();
        let mut line = clean(&line, names);
        if line.chars().count() > MAX_LINE {
            line = line.chars().take(MAX_LINE).collect::<String>() + "…";
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.truncate(out.trim_end().len());
    out
}

/// The names this account and computer are known by (`USERNAME`, `USER`,
/// the profile folder's own name, `COMPUTERNAME`, `USERDOMAIN`), hidden
/// wherever they stand in the report.
pub fn local_names() -> Vec<String> {
    let mut names: Vec<String> = ["USERNAME", "USER", "COMPUTERNAME", "USERDOMAIN"].iter().filter_map(|v| std::env::var(v).ok()).collect();
    names.extend(["USERPROFILE", "HOME"].iter().filter_map(|v| {
        let p = std::env::var(v).ok()?;
        Some(p.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next()?.to_string())
    }));
    names.retain(|n| !n.trim().is_empty());
    names.sort();
    names.dedup();
    names
}

/// [`crate::feedback::anonymize_with`], then [`scrub`].
pub fn clean(text: &str, names: &[&str]) -> String {
    scrub(&crate::feedback::anonymize_with(text, names))
}

/// Replaces with «…» whatever in `text` looks like a path (drive, UNC,
/// `%VAR%\`, `~/`, `/…`, or any word with a backslash), a file name
/// (`name.ext`), an e-mail address, a web address, an IP address (v4 or
/// v6) or `<user>`. Bare drives (`C:\`), numbers, versions and the
/// program's own file names (`bahth.exe`, `index.bin`…) are kept.
pub fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        // `<user>` (from the anonymiser) is held by a placeholder meanwhile:
        // its `<` would end a path.
        let line = scrub_paths(&line.replace("<user>", USER));
        out.push_str(&scrub_words(&line).replace(USER, GONE));
    }
    out
}

/// Stands for `<user>` while a line is filtered.
const USER: &str = "\u{E000}";

fn is_sep(c: char) -> bool {
    c == '\\' || c == '/'
}

/// Characters a path never holds on Windows (with the line breaks and the
/// quotes that close one).
const HARD_END: [char; 12] = ['"', '<', '>', '|', '*', '?', '\t', '\r', '\n', '”', '»', '“'];

/// What may follow a bare drive (`C:\`) for it to stay one.
const AFTER_ROOT: [char; 12] = [':', ';', ',', '.', ')', ']', '}', '\'', '"', '»', '”', '>'];

enum Start {
    /// A bare drive of this many bytes, kept.
    Root(usize),
    /// A path starts here.
    Path,
}

/// Whether a path (or a bare drive) starts at byte `i` of `line`.
fn path_start(line: &str, i: usize, prev: Option<char>) -> Option<Start> {
    let b = line.as_bytes();
    let at = |k: usize| b.get(i + k).copied().unwrap_or(0);
    let word_start = prev.is_none_or(|c| c.is_whitespace() || "([{'\"«“‘<`=".contains(c));
    // C:\ or C:/
    if at(0).is_ascii_alphabetic() && at(1) == b':' && (at(2) == b'\\' || at(2) == b'/') && !prev.is_some_and(|c| c.is_alphanumeric() || c == '_') {
        let next = line[i + 3..].chars().next();
        return Some(if next.is_none_or(|c| c.is_whitespace() || AFTER_ROOT.contains(&c)) { Start::Root(3) } else { Start::Path });
    }
    // \\server\share, \\?\C:\…
    if at(0) == b'\\' && at(1) == b'\\' && prev != Some('\\') && at(2) != 0 && !(at(2) as char).is_whitespace() {
        return Some(Start::Path);
    }
    // %LOCALAPPDATA%\…
    if at(0) == b'%' {
        let rest = &line[i + 1..];
        if let Some(end) = rest.find('%')
            && end > 0
            && end < 40
            && rest[..end].bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'(' || c == b')')
            && rest[end + 1..].starts_with(is_sep)
        {
            return Some(Start::Path);
        }
    }
    // ~/… or ~\…, and /… at the start of a word
    if word_start && at(0) == b'~' && (at(1) == b'/' || at(1) == b'\\') {
        return Some(Start::Path);
    }
    if word_start && at(0) == b'/' && line[i + 1..].chars().next().is_some_and(char::is_alphanumeric) {
        return Some(Start::Path);
    }
    None
}

/// Where the path that starts at byte `start` ends (exclusive); `prev` is
/// the character before it, whose closing partner ends it too.
fn path_end(line: &str, start: usize, prev: Option<char>) -> usize {
    let closer = match prev {
        Some('(') => Some(')'),
        Some('[') => Some(']'),
        Some('{') => Some('}'),
        Some('\'') => Some('\''),
        Some('`') => Some('`'),
        Some('‘') => Some('’'),
        _ => None,
    };
    // The `?` and `.` of `\\?\` and `\\.\` are part of the path.
    let skip = if line[start..].starts_with("\\\\?\\") || line[start..].starts_with("\\\\.\\") { 4 } else { 0 };
    let mut end = line.len();
    let mut it = line[start + skip..].char_indices().peekable();
    let mut depth = 0u32;
    while let Some((k, c)) = it.next() {
        let here = start + skip + k;
        let next = it.peek().map(|&(_, n)| n);
        let after = &line[here + c.len_utf8()..];
        // Inside parentheses, a name's own "(1)" does not end the path.
        if closer == Some(')') && c == '(' {
            depth += 1;
            continue;
        }
        if closer == Some(')') && c == ')' && depth > 0 {
            depth -= 1;
            continue;
        }
        let ends = Some(c) == closer
            || HARD_END.contains(&c)
            || c == ';'
            || (c == ':' && next.is_none_or(char::is_whitespace))
            || (c == ',' && next.is_none_or(char::is_whitespace))
            || (c.is_whitespace()
                && ((after.starts_with('(') && closer != Some(')'))
                    || after.starts_with("- ")
                    || after.starts_with("— ")
                    || after.starts_with("\\\\")
                    || matches!(after.as_bytes(), [d, b':', s, ..] if d.is_ascii_alphabetic() && (*s == b'\\' || *s == b'/'))));
        if ends {
            end = here;
            break;
        }
    }
    let mut span = line[start..end].trim_end_matches(|c: char| c.is_whitespace() || c == '.' || c == ',');
    // The last part may hold spaces ("a b.pdf"): it ends after its last
    // word that looks like a file name, when there is one.
    let last = span.rfind(is_sep).map_or(0, |i| i + 1);
    let tail = &span[last..];
    if tail.contains(char::is_whitespace) {
        let mut cut = None;
        let mut pos = 0;
        for word in tail.split_inclusive(char::is_whitespace) {
            let w = word.trim_end();
            if looks_like_file(w.trim_end_matches(|c: char| ".,;:)".contains(c))) {
                cut = Some(pos + w.len());
            }
            pos += word.len();
        }
        if let Some(cut) = cut {
            span = &span[..last + cut];
        }
    }
    start + span.len().max(1)
}

/// Pass one: absolute paths, which may hold spaces.
fn scrub_paths(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let (mut i, mut kept) = (0, 0);
    while i < line.len() {
        let prev = line[..i].chars().next_back();
        match path_start(line, i, prev) {
            Some(Start::Root(n)) => i += n,
            Some(Start::Path) => {
                let end = path_end(line, i, prev);
                out.push_str(&line[kept..i]);
                out.push_str(GONE);
                (i, kept) = (end, end);
            }
            None => i += line[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out.push_str(&line[kept..]);
    out
}

/// Punctuation around a word that is not part of what it names.
fn is_wrap(c: char) -> bool {
    "()[]{}<>\"'`«»“”‘’,;:!?".contains(c) || (c == '.')
}

/// Pass two: word by word, what has no spaces (file names, addresses,
/// relative paths).
fn scrub_words(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for word in line.split_inclusive(char::is_whitespace) {
        let body = word.trim_end_matches(char::is_whitespace);
        let space = &word[body.len()..];
        let core = body.trim_start_matches(is_wrap);
        let lead = &body[..body.len() - core.len()];
        let core = core.trim_end_matches(is_wrap);
        let trail = &body[lead.len() + core.len()..];
        if sensitive(core) {
            out.push_str(lead);
            out.push_str(GONE);
            out.push_str(trail);
        } else {
            out.push_str(body);
        }
        out.push_str(space);
    }
    out
}

fn sensitive(core: &str) -> bool {
    if core.is_empty() || core == "…" || core.contains(GONE) && core.replace(GONE, "").chars().all(|c| is_wrap(c) || is_sep(c)) {
        return false;
    }
    if let Some(at) = core.find('@')
        && at > 0
        && at + 1 < core.len()
    {
        return true;
    }
    let lower = core.to_ascii_lowercase();
    if core.contains("://") || lower.starts_with("www.") {
        return true;
    }
    // A bare drive: `C:\`, `D:/`.
    if matches!(core.as_bytes(), [d, b':', b'\\' | b'/'] if d.is_ascii_alphabetic()) {
        return false;
    }
    if core.contains('\\') {
        return !is_source(core);
    }
    if core.contains('/') {
        return core.split('/').any(|p| looks_like_file(p) && !own_name(p)) && !is_source(core);
    }
    if is_ipv4(core) || is_ipv6(core) {
        return true;
    }
    looks_like_file(core) && !own_name(core) && !is_source(core)
}

/// `name.ext`: an extension of 1 to 10 ASCII letters and digits starting
/// with a letter, after a name.
fn looks_like_file(s: &str) -> bool {
    let Some(dot) = s.rfind('.') else { return false };
    let (name, ext) = (&s[..dot], &s[dot + 1..]);
    !name.is_empty()
        && !name.ends_with('.')
        && (1..=10).contains(&ext.len())
        && ext.bytes().all(|c| c.is_ascii_alphanumeric())
        && ext.as_bytes()[0].is_ascii_alphabetic()
}

/// The program's own files, named in its log lines.
fn own_name(s: &str) -> bool {
    const OWN: [&str; 12] = [
        "bahth.exe",
        "bahth-x86.exe",
        "bahth-arm64.exe",
        "index.bin",
        "log.txt",
        "crash.txt",
        "settings.txt",
        "history.json",
        "content.dat",
        "content.idx",
        "explorer.exe",
        "msedgewebview2.exe",
    ];
    let lower = s.to_ascii_lowercase();
    OWN.iter().any(|o| lower.strip_prefix(o).is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '+'])))
}

/// A place in the program's own source (`src\search.rs:12:5`), named by a
/// panic: relative, ending in `.rs`, perhaps with a line and column.
fn is_source(s: &str) -> bool {
    let mut t = s;
    while let Some((head, n)) = t.rsplit_once(':') {
        if n.is_empty() || !n.bytes().all(|c| c.is_ascii_digit()) {
            break;
        }
        t = head;
    }
    t.ends_with(".rs") && !t.starts_with(is_sep) && !t.contains(':')
}

/// `192.168.1.20`, perhaps with `:port` or `/prefix`.
fn is_ipv4(s: &str) -> bool {
    let s = s.split([':', '/']).next().unwrap_or(s);
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255))
}

/// `fe80::1%12`, `2001:db8:0:0:0:0:0:1`: hex groups of at most four,
/// with `::` or seven colons (so a time such as `12:30:45` is not one).
fn is_ipv6(s: &str) -> bool {
    let s = s.split(['%', ']']).next().unwrap_or(s);
    let colons = s.matches(':').count();
    if colons < 2 || !(s.contains("::") || colons >= 7) {
        return false;
    }
    let mut groups = s.split(':').filter(|g| !g.is_empty()).peekable();
    groups.peek().is_some()
        && groups.all(|g| (g.len() <= 4 && g.bytes().all(|c| c.is_ascii_hexdigit())) || is_ipv4(g))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: &str) -> String {
        clean(t, &[])
    }

    #[test]
    fn paths_in_many_shapes_are_removed() {
        let cases = [
            ("open C:\\Users\\Saif\\Desktop\\x.pdf failed", "open «…» failed"),
            ("read D:\\Work\\Q3 report final.docx: Access is denied", "read «…»: Access is denied"),
            ("at C:/Users/ana/x.txt", "at «…»"),
            ("share \\\\server\\Users\\Saif Ali (Work)\\a.xlsx gone", "share «…» gone"),
            ("unc \\\\nas\\photos\\2024\\img 001.jpg; next", "unc «…»; next"),
            ("long \\\\?\\C:\\very\\deep\\file.bin done", "long «…» done"),
            ("data %LOCALAPPDATA%\\BahthSari\\content\\content.dat ok", "data «…» ok"),
            ("home ~/Documents/notes.md here", "home «…» here"),
            ("mac /Users/ana/Desktop/a.pdf here", "mac «…» here"),
            ("قرأ C:\\Users\\سيف علي\\مستندات\\تقرير الربع الأول.pdf بنجاح", "قرأ «…» بنجاح"),
            ("ملف E:\\أرشيف\\٢٠٢٤\\فاتورة.xlsx: تالف", "ملف «…»: تالف"),
            ("running from Downloads (C:\\Users\\x\\Downloads\\bahth (1).exe)", "running from Downloads («…»)"),
            ("copied to C:\\Users\\x\\AppData\\Local\\Programs\\InstaFind; restarting", "copied to «…»; restarting"),
            ("cannot write to C:\\Program Files\\InstaFind", "cannot write to «…»"),
            ("in 'C:\\Users\\O'Neil Smith\\a.txt' now", "in '«…»' now"),
            ("\"F:\\My Music\\song.mp3\" played", "\"«…»\" played"),
            ("relative Documents\\x\\y.pdf too", "relative «…» too"),
            ("two C:\\a\\b.txt D:\\c\\d.txt", "two «…» «…»"),
            ("folder C:\\Some Folder: denied", "folder «…»: denied"),
            ("C:\\x", "«…»"),
        ];
        for (input, want) in cases {
            assert_eq!(s(input), want, "{input}");
        }
    }

    #[test]
    fn file_names_addresses_and_names_are_removed() {
        let cases = [
            ("skipped تقرير.pdf (too big)", "skipped «…» (too big)"),
            ("opened photo.JPEG, then notes.txt.", "opened «…», then «…»."),
            ("archive.tar.gz and 2024.xlsx", "«…» and «…»"),
            ("mail saif.ali@example.com please", "mail «…» please"),
            ("from <saif@example.org>", "from <«…»>"),
            ("web: listening on 192.168.1.20:8080 and 10.0.0.5", "web: listening on «…» and «…»"),
            ("refused a message from https://evil.example/x?y=1", "refused a message from «…»"),
            ("ipv6 fe80::1c2b:3d4e%12 and [2001:db8::1]:80", "ipv6 «…» and [«…»"),
            ("see www.example.com now", "see «…» now"),
        ];
        for (input, want) in cases {
            assert_eq!(s(input), want, "{input}");
        }
        // User names, with spaces, wherever they are known.
        let names = ["Saif Ali", "saif", "DESKTOP-7H2K"];
        assert_eq!(clean("C:\\Users\\Saif Ali\\x.txt by Saif Ali on DESKTOP-7H2K", &names), "«…» by «…» on «…»");
        assert_eq!(clean("C:\\Users\\Saif Ali: denied", &[]), "«…»: denied");
        // A folder with spaces and no file name after it: the rest of the
        // line goes with it, rather than leaking part of a name.
        assert_eq!(clean("in C:\\Data\\Saif Ali stuff 3 ms", &[]), "in «…»");
        assert_eq!(clean("user saif logged", &names), "user «…» logged");
    }

    /// The 1.0 memory, save and handover lines hold numbers and drives
    /// only: they come through unchanged.
    #[test]
    fn the_1_0_log_lines_are_kept() {
        let sample = crate::mem_guard::Sample { private: 5161 << 20, entries: 4_000_000, documents: 3_000_000, queue: 12 };
        let lines = [
            format!("1731400000 {}", sample.line()),
            format!("1731400030 memory: passed 4 GB; {}", sample.line()),
            "1731400031 memory guard: private 5161 MB is over 3x the expected 1500 MB; stopped watching, dropped 12 waiting changes and the caches; watching again in 30 min".into(),
            "1731402000 memory guard: watching again; walking C:\\ D:\\ (paced)".into(),
            "1731402001 memory guard: not watching again now (watching is off, or the index is being scanned or loaded)".into(),
            "1731400100 index save: started gen 42".into(),
            "1731400101 index save: done gen 42 in 1830 ms (4000000 entries; the next one waits 18 s) [3 more since the last such line]".into(),
            "1731400102 index save: skipped gen 41 (gen 42 is already on disk)".into(),
            "1731400103 index save: failed gen 43 after 12 ms: the save panicked; trying again in 30 s".into(),
            "1731400104 index save: not finished within 3000 ms on exit; the file on disk stays as it was".into(),
            "1731400105 exit: last index save Done after 120 ms (Session)".into(),
            "1731400106 handover: previous copy (pid 4242) ended after 310 ms".into(),
            "1731400107 handover: previous copy (pid 4242) still running after 10 s; ended it".into(),
            "1731400108 handover (update): new copy (pid 5151) is running; leaving".into(),
            "1731400109 rescan C:\\: 812345 entries in 5400 ms".into(),
            "1731400110 rescan D:\\: done; the next one waits at least 10 min".into(),
            "1731400111 startup ms (v1.0.0): window=120 index=340".into(),
            "1731400112 webui: WebView2 131.0.2903.112".into(),
            "1731400113 content index open: 300000 documents, 1048576 KB in 950 ms".into(),
            "1731400114 update leftovers: removed bahth.exe.old".into(),
            "1731400115 PANIC (v1.0.0) in thread 'main' at src\\search.rs:120:5: index out of bounds".into(),
            "1731400116 native crash: exception 0xC00000FD at bahth.exe+0x1A2B".into(),
            "1731400117 web: listening on 2 address(es), port 8080".into(),
        ];
        for line in &lines {
            assert_eq!(&s(line), line);
        }
    }

    #[test]
    fn the_log_tail_is_short_and_clean() {
        let log: String = (0..200).map(|i| format!("{i} line {i} C:\\Users\\a\\f{i}.txt\n")).collect();
        let tail = log_tail(&log, 80, &[]);
        assert_eq!(tail.lines().count(), 80);
        assert!(tail.starts_with("120 line 120 «…»"), "{tail}");
        assert!(!tail.contains("f199") && !tail.contains("Users"));
        let long = format!("1 {}", "x ".repeat(1000));
        assert_eq!(log_tail(&long, 80, &[]).chars().count(), MAX_LINE + 1);
        assert_eq!(log_tail("", 80, &[]), "");
        assert_eq!(log_tail("a\r\n\r\nb\u{7}c\r\n", 80, &[]), "a\nbc");
    }

    fn facts() -> Facts {
        Facts {
            version: "1.0.0".into(),
            os: "Windows 11 24H2 (build 26100.2314)".into(),
            arch: "x86_64".into(),
            lang: "ar".into(),
            build: "winget".into(),
            new_look: true,
            new_look_chosen: true,
            entries: 4_000_000,
            documents: Some(300_000),
            index_bytes: Some(512 << 20),
            content_bytes: Some(3 << 30),
            private_bytes: Some(900 << 20),
            peak_bytes: Some(5161 << 20),
            uptime_secs: 3 * 3600 + 12 * 60 + 5,
            auto_index: true,
            content_index: true,
            phone_on: false,
            phone_running: false,
        }
    }

    #[test]
    fn the_report_states_the_facts() {
        let log = "1 memory: private 900 MB, entries 4000000, documents 300000, queue 0 (expected about 1600 MB)\n2 opened C:\\Users\\Saif\\a.pdf\n";
        let r = build(&facts(), "  It used 5 GB after I opened D:\\Work\\big.pdf  ", log, &["Saif"]);
        assert_eq!(r.what, "It used 5 GB after I opened «…»");
        let d = &r.details;
        for want in [
            "InstaFind 1.0.0 on Windows 11 24H2 (build 26100.2314) (x86_64), interface ar",
            "Installed: winget",
            "Look: new",
            "Index: 4000000 files and folders; content index: 300000 documents",
            "Data files: index.bin 512 MB; content store 3072 MB",
            "Memory: private 900 MB now, peak 5161 MB",
            "Running for: 3 h 12 min",
            "Settings: update the index automatically on, content index on, phone search off",
            "Last 2 lines of log.txt (file names and paths removed):\n1 memory: private 900 MB",
            "2 opened «…»",
        ] {
            assert!(d.contains(want), "{want}\n---\n{d}");
        }
        assert!(!d.contains("Saif") && !d.contains("a.pdf"));
        assert!(r.text().starts_with("What happened:\nIt used 5 GB"));
        assert!(r.text().ends_with(d.as_str()));

        let f = Facts { new_look: false, documents: None, index_bytes: None, uptime_secs: 40, phone_on: true, ..facts() };
        let r = build(&f, "", "", &[]);
        assert!(r.text().starts_with("What happened:\n-\n\n"));
        assert!(r.details.contains("Look: classic (the new look is chosen but not running)"));
        assert!(r.details.contains("content index: not open documents"));
        assert!(r.details.contains("index.bin unknown"));
        assert!(r.details.contains("Running for: 40 s"));
        assert!(r.details.contains("phone search on (not running now)"));
        assert!(r.details.ends_with("Last 0 lines of log.txt (file names and paths removed):\n(empty)"));
        // The user's words are bounded.
        assert_eq!(build(&f, &"ب".repeat(5000), "", &[]).what.chars().count(), MAX_WHAT);
    }
}
