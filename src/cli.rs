//! Command-line interface, in the same exe as the GUI (like Everything's es.exe):
//! `bahth.exe -s "فاتورة ext:pdf" -n 20 --sort date --desc`.
//!
//! Argument parsing, the search and the output formats are platform
//! independent and tested everywhere; only attaching to the parent console
//! and writing to it are Windows-only.

use std::io::{self, Write};

use crate::index::{Entry, Index};
use crate::search::{Query, SortKey, search_names, sort_hits};

/// Exit codes: results found, no results, error (bad arguments, no index).
pub const EXIT_FOUND: i32 = 0;
pub const EXIT_NONE: i32 = 1;
pub const EXIT_ERROR: i32 = 2;

/// Every option the CLI knows, with whether it takes a value.
const OPTIONS: &[(&str, bool)] = &[
    ("-s", true),
    ("--search", true),
    ("-n", true),
    ("--max", true),
    ("--sort", true),
    ("--desc", false),
    ("--scope", true),
    ("--count", false),
    ("--csv", false),
    ("--json", false),
    ("-h", false),
    ("--help", false),
    ("/?", false),
    ("--version", false),
];

/// True when `arg` (the first command-line argument) asks for the CLI rather
/// than the GUI. Anything else, `--updated` included, starts the GUI.
pub fn is_cli_arg(arg: &str) -> bool {
    let name = arg.split_once('=').map_or(arg, |(n, _)| n);
    OPTIONS.iter().any(|&(o, _)| o == name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Full paths, one per line.
    #[default]
    Paths,
    /// Only the number of results.
    Count,
    Csv,
    Json,
}

#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub query: String,
    pub max: Option<usize>,
    pub sort: SortKey,
    pub desc: bool,
    pub scope: String,
    pub format: Format,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Version,
    Search(Options),
}

/// Parses the arguments after the program name. Options may be given as
/// `--max 5` or `--max=5`; words that are not options join the query, and
/// everything after `--` is query text.
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut o = Options::default();
    let mut words: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let arg = arg.as_str();
        if arg == "--" {
            words.extend(it.by_ref().map(String::as_str));
            break;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v)),
            _ => (arg, None),
        };
        let Some(&(name, takes_value)) = OPTIONS.iter().find(|&&(o, _)| o == name) else {
            // `-word` is an exclusion in the query; only `--name` must be an option.
            if arg.starts_with("--") {
                return Err(format!("unknown option: {arg}"));
            }
            words.push(arg);
            continue;
        };
        let value = if takes_value {
            match inline.or_else(|| it.next().map(String::as_str)) {
                Some(v) => v,
                None => return Err(format!("{name} needs a value")),
            }
        } else {
            if inline.is_some() {
                return Err(format!("{name} takes no value"));
            }
            ""
        };
        match name {
            "-s" | "--search" => words.push(value),
            "-n" | "--max" => {
                let n = value.trim().parse::<usize>().map_err(|_| format!("{name}: not a number: {value}"))?;
                o.max = Some(n);
            }
            "--sort" => {
                o.sort = match value.to_ascii_lowercase().as_str() {
                    "name" => SortKey::Name,
                    "path" => SortKey::Path,
                    "size" => SortKey::Size,
                    "date" => SortKey::Date,
                    _ => return Err(format!("--sort: expected name, path, size or date, got {value}")),
                }
            }
            "--desc" => o.desc = true,
            "--scope" => o.scope = scope_prefix(value),
            "--count" | "--csv" | "--json" => {
                let f = match name {
                    "--count" => Format::Count,
                    "--csv" => Format::Csv,
                    _ => Format::Json,
                };
                if o.format != Format::Paths && o.format != f {
                    return Err("choose only one of --count, --csv and --json".into());
                }
                o.format = f;
            }
            "-h" | "--help" | "/?" => return Ok(Command::Help),
            "--version" => return Ok(Command::Version),
            _ => unreachable!("every option in OPTIONS is handled"),
        }
    }
    o.query = words.join(" ");
    if Query::parse(&o.query).is_empty() {
        return Err("no search text; use -s \"words\"".into());
    }
    Ok(Command::Search(o))
}

/// A folder as a path prefix: forward slashes become backslashes and one
/// trailing backslash is kept, so `C:\Users` does not also match `C:\Users2`.
fn scope_prefix(folder: &str) -> String {
    let s = folder.trim().replace('/', "\\");
    let s = s.trim_end_matches('\\');
    if s.is_empty() { String::new() } else { format!("{s}\\") }
}

pub fn usage() -> String {
    format!(
        "بحث سريع {v} — البحث من سطر الأوامر
Bahth Sari' {v} — command-line search

الاستخدام / Usage:
  bahth.exe -s \"فاتورة ext:pdf\" [options]

  -s, --search <text>   نص البحث بنفس صيغة البرنامج / search text, same syntax as the GUI
  -n, --max <N>         أقصى عدد للنتائج / at most N results
  --sort <key>          الترتيب: name | path | size | date / sort order
  --desc                ترتيب تنازلي / descending order
  --scope <folder>      البحث داخل مجلد فقط / only inside this folder
  --count               اطبع عدد النتائج فقط / print only the number of results
  --csv                 path,size,modified (CSV)
  --json                مصفوفة JSON / a JSON array
  -h, --help            هذه المساعدة / this help
  --version             رقم الإصدار / version

رموز الخروج / Exit codes: 0 نتائج / results, 1 لا نتائج / none, 2 خطأ / error
يستخدم الفهرس المحفوظ؛ افتح البرنامج مرة لبنائه أو تحديثه.
Uses the saved index; open the program once to build or refresh it.
",
        v = env!("CARGO_PKG_VERSION")
    )
}

pub const NO_INDEX_HINT: &str = "لا يوجد فهرس بعد: افتح برنامج «بحث سريع» مرة واحدة لبنائه، ثم أعد المحاولة.
No index yet: open Bahth Sari' once to build it, then try again.
";

/// Windows line endings, like the rest of the output.
fn crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

/// Runs a parsed command, writing results to `out` and messages to `err`,
/// and returns the exit code. `load` is only called for a search.
pub fn execute(cmd: &Command, load: impl FnOnce() -> Option<Index>, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<i32> {
    let o = match cmd {
        Command::Help => {
            out.write_all(crlf(&usage()).as_bytes())?;
            return Ok(EXIT_FOUND);
        }
        Command::Version => {
            write!(out, "bahth {}\r\n", env!("CARGO_PKG_VERSION"))?;
            return Ok(EXIT_FOUND);
        }
        Command::Search(o) => o,
    };
    let Some(index) = load() else {
        err.write_all(crlf(NO_INDEX_HINT).as_bytes())?;
        return Ok(EXIT_ERROR);
    };
    let hits = find(&index.entries, o);
    write_hits(&index.entries, &hits, o.format, out)?;
    Ok(if hits.is_empty() { EXIT_NONE } else { EXIT_FOUND })
}

/// Searches, sorts and limits as the options say.
fn find(entries: &[Entry], o: &Options) -> Vec<u32> {
    let mut hits = search_names(entries, &Query::parse(&o.query), &o.scope);
    if o.sort == SortKey::Relevance {
        if o.desc {
            hits.reverse();
        }
    } else {
        sort_hits(entries, &mut hits, o.sort, o.desc);
    }
    if let Some(n) = o.max {
        hits.truncate(n);
    }
    hits
}

fn write_hits(entries: &[Entry], hits: &[u32], format: Format, out: &mut dyn Write) -> io::Result<()> {
    let entries = hits.iter().filter_map(|&i| entries.get(i as usize));
    match format {
        Format::Count => write!(out, "{}\r\n", hits.len())?,
        Format::Paths => {
            for e in entries {
                write!(out, "{}\r\n", e.path)?;
            }
        }
        Format::Csv => {
            out.write_all(b"path,size,modified\r\n")?;
            for e in entries {
                let size = if e.is_dir { String::new() } else { e.size.to_string() };
                let fields = [csv_field(&e.path), size, iso_time(e.mtime)];
                write!(out, "{}\r\n", fields.join(","))?;
            }
        }
        Format::Json => {
            out.write_all(b"[")?;
            #[derive(serde::Serialize)]
            struct Item<'a> {
                path: &'a str,
                name: &'a str,
                size: Option<u64>,
                modified: String,
                is_dir: bool,
            }
            for (n, e) in entries.enumerate() {
                let item = Item {
                    path: &e.path,
                    name: e.name(),
                    size: (!e.is_dir).then_some(e.size),
                    modified: iso_time(e.mtime),
                    is_dir: e.is_dir,
                };
                let obj = serde_json::to_string(&item).map_err(io::Error::other)?;
                write!(out, "{}\r\n  {obj}", if n == 0 { "" } else { "," })?;
            }
            out.write_all(if hits.is_empty() { b"]\r\n" } else { b"\r\n]\r\n" })?;
        }
    }
    out.flush()
}

/// Quotes a CSV cell, and stops Excel from reading a cell that starts with
/// `= + - @` as a formula by prefixing an apostrophe. Same rules as the
/// GUI's CSV export (`ui::format::csv_field`).
pub fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

/// Seconds since the Unix epoch as an ISO 8601 UTC time,
/// `2024-03-05T14:07:09Z`; empty for an unknown (zero or negative) time.
pub fn iso_time(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Entry point from `main` when the first argument is a CLI option: attaches
/// to the console of the shell that started us, runs the command and exits
/// with its code. Never touches the GUI's single-instance mutex or windows.
#[cfg(windows)]
pub fn run(args: &[String]) -> ! {
    let attached = console::attach();
    let code = match console::Writer::pair(attached) {
        Some((mut out, mut err)) => match parse_args(args) {
            Ok(cmd) => {
                let load = || Index::load(&crate::index::cache_path());
                execute(&cmd, load, &mut out, &mut err).unwrap_or(EXIT_ERROR)
            }
            Err(msg) => {
                let _ = write!(err, "خطأ / error: {msg}\r\nbahth.exe --help\r\n");
                EXIT_ERROR
            }
        },
        // Started from Explorer or similar: there is nowhere to write to.
        None => EXIT_ERROR,
    };
    // The writers were dropped above, which flushes them; `exit` runs no destructors.
    console::detach(attached);
    std::process::exit(code)
}

#[cfg(windows)]
mod console {
    //! Writing UTF-8 to the parent console (as UTF-16, so Arabic survives
    //! any code page) or, when redirected, raw bytes to the file or pipe.

    use std::io::{self, Write};
    use std::ptr::null_mut;

    use winapi::shared::minwindef::DWORD;
    use winapi::um::consoleapi::{GetConsoleMode, WriteConsoleW};
    use winapi::um::fileapi::{CreateFileW, GetFileType, OPEN_EXISTING, WriteFile};
    use winapi::um::handleapi::INVALID_HANDLE_VALUE;
    use winapi::um::processenv::GetStdHandle;
    use winapi::um::winbase::{FILE_TYPE_CHAR, FILE_TYPE_UNKNOWN, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    use winapi::um::wincon::{
        ATTACH_PARENT_PROCESS, AttachConsole, CONSOLE_SCREEN_BUFFER_INFO, FreeConsole, GetConsoleScreenBufferInfo,
    };
    use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE, HANDLE};

    /// Attaches to the parent's console; false when it has none.
    pub fn attach() -> bool {
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
    }

    pub fn detach(attached: bool) {
        if attached {
            unsafe { FreeConsole() };
        }
    }

    pub struct Writer {
        handle: HANDLE,
        console: bool,
        /// Bytes not yet written; a UTF-8 sequence may be split across writes.
        buf: Vec<u8>,
        /// Whether anything was written to a console, and the last byte.
        last: Option<u8>,
    }

    impl Writer {
        /// Writers for stdout and stderr. A handle the parent redirected to a
        /// file or pipe is used as is, even without a console (e.g. a pipe
        /// from a GUI program). Otherwise the console is used, opened as
        /// `CONOUT$` when the std handles were not set for us. `None` when
        /// there is no console and nothing redirected.
        pub fn pair(attached: bool) -> Option<(Writer, Writer)> {
            let conout = if attached { open_conout() } else { None };
            let pick = |std: DWORD| {
                let h = unsafe { GetStdHandle(std) };
                let usable = !h.is_null() && h != INVALID_HANDLE_VALUE && unsafe { GetFileType(h) } != FILE_TYPE_UNKNOWN;
                if usable { Some(h) } else { conout }
            };
            let out = pick(STD_OUTPUT_HANDLE)?;
            let err = pick(STD_ERROR_HANDLE).unwrap_or(out);
            Some((Writer::new(out), Writer::new(err)))
        }

        fn new(handle: HANDLE) -> Writer {
            let mut mode = 0;
            let console = unsafe { GetFileType(handle) } == FILE_TYPE_CHAR && unsafe { GetConsoleMode(handle, &mut mode) } != 0;
            let mut w = Writer { handle, console, buf: Vec::with_capacity(64 * 1024), last: None };
            // The shell does not wait for a GUI program, so its prompt may
            // already be on the cursor's line: start on a fresh line.
            if console {
                let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
                if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } != 0 && info.dwCursorPosition.X != 0 {
                    w.buf.extend_from_slice(b"\r\n");
                }
            }
            w
        }

        fn write_out(&mut self, all: bool) -> io::Result<()> {
            if self.buf.is_empty() {
                return Ok(());
            }
            let n = if self.console {
                let valid = match std::str::from_utf8(&self.buf) {
                    Ok(s) => s.len(),
                    // Keep an incomplete trailing sequence for the next write.
                    Err(e) if e.error_len().is_none() && !all => e.valid_up_to(),
                    Err(_) => self.buf.len(),
                };
                let text = String::from_utf8_lossy(&self.buf[..valid]);
                let wide: Vec<u16> = text.encode_utf16().collect();
                for chunk in wide.chunks(8192) {
                    let mut done = 0;
                    if unsafe { WriteConsoleW(self.handle, chunk.as_ptr().cast(), chunk.len() as DWORD, &mut done, null_mut()) } == 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                valid
            } else {
                let mut off = 0;
                while off < self.buf.len() {
                    let part = &self.buf[off..(off + (1 << 20)).min(self.buf.len())];
                    let mut done = 0;
                    let ok = unsafe { WriteFile(self.handle, part.as_ptr().cast(), part.len() as DWORD, &mut done, null_mut()) };
                    if ok == 0 || done == 0 {
                        return Err(io::Error::last_os_error());
                    }
                    off += done as usize;
                }
                off
            };
            if n > 0 {
                self.last = Some(self.buf[n - 1]);
            }
            self.buf.drain(..n);
            Ok(())
        }
    }

    impl Write for Writer {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.buf.extend_from_slice(data);
            if self.buf.len() >= 64 * 1024 {
                self.write_out(false)?;
            }
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.write_out(true)
        }
    }

    impl Drop for Writer {
        fn drop(&mut self) {
            // Leave the console cursor at the start of a line for the prompt.
            if self.console && self.last.is_some_and(|b| b != b'\n') {
                self.buf.extend_from_slice(b"\r\n");
            }
            let _ = self.write_out(true);
        }
    }

    fn open_conout() -> Option<HANDLE> {
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let h = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        (h != INVALID_HANDLE_VALUE).then_some(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn search(a: &[&str]) -> Options {
        match parse_args(&args(a)) {
            Ok(Command::Search(o)) => o,
            other => panic!("expected a search, got {other:?}"),
        }
    }

    fn entry(path: &str, size: u64, mtime: i64, is_dir: bool) -> Entry {
        let mut e = Entry { path: path.into(), size, mtime, is_dir, norm_name: "".into() };
        e.norm_name = crate::normalize::normalize(e.name()).into();
        e
    }

    fn sample() -> Index {
        let mut entries = vec![
            entry(r"C:\docs\فاتورة يناير.pdf", 2048, 1_700_000_000, false),
            entry(r"C:\docs\فاتورة فبراير.docx", 100, 1_710_000_000, false),
            entry(r"C:\old\فاتورة.pdf", 50_000, 1_600_000_000, false),
            entry(r"C:\old\report, final.txt", 7, 1_650_000_000, false),
            entry(r"C:\docs\reports", 0, 1_690_000_000, true),
        ];
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Index { entries, roots: vec![r"C:\".into()], built_at: 0 }
    }

    fn run_cli(a: &[&str], index: Option<Index>) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = match parse_args(&args(a)) {
            Ok(cmd) => execute(&cmd, || index, &mut out, &mut err).unwrap(),
            Err(_) => EXIT_ERROR,
        };
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn dispatch_only_on_cli_flags() {
        for a in ["-s", "--search", "-n", "--max=3", "--count", "--json", "-h", "--help", "--version", "/?"] {
            assert!(is_cli_arg(a), "{a}");
        }
        for a in ["--updated", "", "فاتورة", "-x", "--searchx", r"C:\file.txt"] {
            assert!(!is_cli_arg(a), "{a}");
        }
    }

    #[test]
    fn parses_options() {
        let o = search(&["-s", "فاتورة ext:pdf", "-n", "5", "--sort", "DATE", "--desc", "--scope", "C:/docs/", "--csv"]);
        assert_eq!(
            o,
            Options {
                query: "فاتورة ext:pdf".into(),
                max: Some(5),
                sort: SortKey::Date,
                desc: true,
                scope: r"C:\docs\".into(),
                format: Format::Csv
            }
        );
        let o = search(&["--max=2", "--search=a b", "c", "--", "-d", "--json"]);
        assert_eq!((o.query.as_str(), o.max, o.format), ("a b c -d --json", Some(2), Format::Paths));
        assert_eq!(search(&["-s", "x", "--scope", r"C:\"]).scope, r"C:\");
        assert_eq!(search(&["-s", "x", "--scope", r"D:\a\b"]).scope, r"D:\a\b\");
        assert_eq!(parse_args(&args(&["-s", "x", "--help"])), Ok(Command::Help));
        assert_eq!(parse_args(&args(&["--version"])), Ok(Command::Version));
    }

    #[test]
    fn rejects_bad_args() {
        for a in [
            &["-s"][..],
            &["-s", ""],
            &["--count"],
            &["-s", "x", "-n", "many"],
            &["-s", "x", "-n", "-1"],
            &["-s", "x", "--sort", "colour"],
            &["-s", "x", "--frobnicate"],
            &["-s", "x", "--csv", "--json"],
            &["-s", "x", "--desc=yes"],
        ] {
            assert!(parse_args(&args(a)).is_err(), "{a:?}");
        }
        // A dash inside the query (an exclusion) is not an option.
        assert_eq!(search(&["-s", "عقد", "-قديم", "-old"]).query, "عقد -قديم -old");
    }

    #[test]
    fn prints_paths_and_exit_codes() {
        let (code, out, _) = run_cli(&["-s", "فاتوره ext:pdf", "--sort", "path"], Some(sample()));
        assert_eq!(code, EXIT_FOUND);
        assert_eq!(out, "C:\\docs\\فاتورة يناير.pdf\r\nC:\\old\\فاتورة.pdf\r\n");

        let (code, out, _) = run_cli(&["-s", "فاتورة", "--sort", "size", "--desc", "-n", "2"], Some(sample()));
        assert_eq!(code, EXIT_FOUND);
        assert_eq!(out, "C:\\old\\فاتورة.pdf\r\nC:\\docs\\فاتورة يناير.pdf\r\n");

        let (code, out, _) = run_cli(&["-s", "فاتورة", "--scope", r"C:\old"], Some(sample()));
        assert_eq!((code, out.as_str()), (EXIT_FOUND, "C:\\old\\فاتورة.pdf\r\n"));

        let (code, out, _) = run_cli(&["-s", "nothing-like-this"], Some(sample()));
        assert_eq!((code, out.as_str()), (EXIT_NONE, ""));
    }

    #[test]
    fn counts() {
        assert_eq!(run_cli(&["-s", "فاتورة", "--count"], Some(sample())), (EXIT_FOUND, "3\r\n".into(), String::new()));
        assert_eq!(run_cli(&["-s", "فاتورة", "--count", "-n", "1"], Some(sample())).1, "1\r\n");
        assert_eq!(run_cli(&["-s", "zzz", "--count"], Some(sample())), (EXIT_NONE, "0\r\n".into(), String::new()));
    }

    #[test]
    fn missing_index_is_an_error_with_a_hint() {
        let (code, out, err) = run_cli(&["-s", "x"], None);
        assert_eq!((code, out.as_str(), err.as_str()), (EXIT_ERROR, "", crlf(NO_INDEX_HINT).as_str()));
        assert!(err.contains("افتح") && err.contains("open"));
    }

    #[test]
    fn csv_output() {
        let (_, out, _) = run_cli(&["-s", "report", "--sort", "path", "--csv"], Some(sample()));
        assert_eq!(
            out,
            "path,size,modified\r\nC:\\docs\\reports,,2023-07-22T04:26:40Z\r\n\"C:\\old\\report, final.txt\",7,2022-04-15T05:20:00Z\r\n"
        );
    }

    #[test]
    fn json_output() {
        let (_, out, _) = run_cli(&["-s", "report", "--sort", "path", "--json"], Some(sample()));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let a = v.as_array().unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0]["path"], r"C:\docs\reports");
        assert_eq!(a[0]["is_dir"], true);
        assert!(a[0]["size"].is_null());
        assert_eq!(a[1]["name"], "report, final.txt");
        assert_eq!(a[1]["size"], 7);
        assert_eq!(a[1]["modified"], "2022-04-15T05:20:00Z");
        // Arabic is written as is, not as \u escapes.
        let (_, out, _) = run_cli(&["-s", "يناير", "--json"], Some(sample()));
        assert!(out.contains("فاتورة يناير.pdf"));
        let (code, out, _) = run_cli(&["-s", "zzz", "--json"], Some(sample()));
        assert_eq!((code, out.as_str()), (EXIT_NONE, "[]\r\n"));
    }

    #[test]
    fn csv_field_matches_gui_rules() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("=1+2"), "'=1+2");
        assert_eq!(csv_field("-x,y"), "\"'-x,y\"");
        assert_eq!(csv_field("@a"), "'@a");
        assert_eq!(csv_field("فاتورة"), "فاتورة");
    }

    #[test]
    fn iso_times() {
        assert_eq!(iso_time(0), "");
        assert_eq!(iso_time(-5), "");
        assert_eq!(iso_time(1), "1970-01-01T00:00:01Z");
        assert_eq!(iso_time(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_time(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_time(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn help_and_version() {
        let (code, out, _) = run_cli(&["--help"], None);
        assert_eq!(code, EXIT_FOUND);
        assert!(out.contains("الاستخدام") && out.contains("Usage") && out.contains("--json"));
        let (_, out, _) = run_cli(&["--version"], None);
        assert_eq!(out, format!("bahth {}\r\n", env!("CARGO_PKG_VERSION")));
    }
}
