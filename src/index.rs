//! In-memory index of every file and folder on the selected drives.
//!
//! The index is built by a parallel directory walk and cached on disk, so the
//! next start-up is instant and filename searches never touch the disk.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use crate::normalize::normalize;

/// Bumped whenever the on-disk format changes.
const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub path: Box<str>,
    pub size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: i64,
    pub is_dir: bool,
    /// Normalised file name, rebuilt after loading (not stored).
    #[serde(skip)]
    pub norm_name: Box<str>,
}

impl Entry {
    pub fn name(&self) -> &str {
        file_name(&self.path)
    }

    /// Lower-case extension without the dot, or "" for none.
    pub fn ext(&self) -> String {
        let name = self.name();
        match name.rfind('.') {
            Some(i) if i > 0 && !self.is_dir => name[i + 1..].to_lowercase(),
            _ => String::new(),
        }
    }
}

fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct Index {
    pub entries: Vec<Entry>,
    /// Roots that were scanned, e.g. `C:\`.
    pub roots: Vec<String>,
    /// When the index was built, seconds since the Unix epoch.
    pub built_at: i64,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    index: Index,
}

/// Shared progress counters for a running scan.
#[derive(Default)]
pub struct ScanProgress {
    pub files: AtomicUsize,
    pub cancel: AtomicBool,
}

/// Folders that are never worth indexing and are slow or access-denied.
fn skip_dir(name: &str) -> bool {
    name.eq_ignore_ascii_case("System Volume Information")
        || name.eq_ignore_ascii_case("$Recycle.Bin")
        || name.eq_ignore_ascii_case("$WinREAgent")
        || name.eq_ignore_ascii_case("WinSxS")
        || [".git", ".svn", ".hg", "node_modules", "__pycache__"]
            .iter()
            .any(|d| name.eq_ignore_ascii_case(d))
}

impl Index {
    /// Scans all `roots` in parallel and returns the finished index.
    pub fn build(roots: &[String], progress: &ScanProgress) -> Index {
        let out: Mutex<Vec<Entry>> = Mutex::new(Vec::with_capacity(500_000));
        rayon::scope(|s| {
            for root in roots {
                let out = &out;
                s.spawn(move |s| walk(s, PathBuf::from(root), out, progress));
            }
        });
        let mut entries = out.into_inner().unwrap_or_default();
        entries.par_sort_unstable_by(|a, b| a.path.cmp(&b.path));
        entries.par_iter_mut().for_each(|e| e.norm_name = normalize(e.name()).into());
        Index { entries, roots: roots.to_vec(), built_at: now_secs() }
    }

    pub fn load(path: &Path) -> Option<Index> {
        let f = fs::File::open(path).ok()?;
        let cache: CacheFile = bincode::deserialize_from(BufReader::new(f)).ok()?;
        if cache.version != CACHE_VERSION {
            return None;
        }
        let mut index = cache.index;
        index
            .entries
            .par_iter_mut()
            .for_each(|e| e.norm_name = normalize(file_name(&e.path)).into());
        Some(index)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Write to a temp file first so a crash never leaves a broken cache.
        let tmp = path.with_extension("tmp");
        {
            let f = BufWriter::new(fs::File::create(&tmp)?);
            #[derive(Serialize)]
            struct CacheRef<'a> {
                version: u32,
                index: &'a Index,
            }
            bincode::serialize_into(f, &CacheRef { version: CACHE_VERSION, index: self })
                .map_err(std::io::Error::other)?;
        }
        fs::rename(tmp, path)
    }
}

fn walk<'s>(s: &rayon::Scope<'s>, dir: PathBuf, out: &'s Mutex<Vec<Entry>>, progress: &'s ScanProgress) {
    if progress.cancel.load(Ordering::Relaxed) {
        return;
    }
    let Ok(rd) = fs::read_dir(&dir) else { return };
    let mut local = Vec::new();
    for ent in rd.flatten() {
        // On Windows file_type() and metadata() come straight from the
        // directory listing, so they cost no extra system call.
        let Ok(ft) = ent.file_type() else { continue };
        // Symlinks and junctions are skipped to avoid loops and duplicates.
        if ft.is_symlink() {
            continue;
        }
        let path = ent.path();
        let Some(path_str) = path.to_str() else { continue };
        let (size, mtime) = match ent.metadata() {
            Ok(m) => (
                if ft.is_dir() { 0 } else { m.len() },
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64),
            ),
            Err(_) => (0, 0),
        };
        local.push(Entry {
            path: path_str.into(),
            size,
            mtime,
            is_dir: ft.is_dir(),
            norm_name: Box::default(),
        });
        if ft.is_dir() && !ent.file_name().to_str().is_some_and(skip_dir) {
            s.spawn(move |s| walk(s, path, out, progress));
        }
    }
    progress.files.fetch_add(local.len(), Ordering::Relaxed);
    if let Ok(mut v) = out.lock() {
        v.append(&mut local);
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Where the index cache lives: `%LOCALAPPDATA%\BahthSari\index.bin`.
pub fn cache_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("BahthSari").join("index.bin")
}

/// Local fixed and removable drives, e.g. `["C:\\", "D:\\"]`.
#[cfg(windows)]
pub fn list_drives() -> Vec<String> {
    use winapi::um::fileapi::{GetDriveTypeW, GetLogicalDrives};
    use winapi::um::winbase::{DRIVE_FIXED, DRIVE_REMOVABLE};
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .filter(|root| {
            let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
            let t = unsafe { GetDriveTypeW(wide.as_ptr()) };
            t == DRIVE_FIXED || t == DRIVE_REMOVABLE
        })
        .collect()
}

#[cfg(not(windows))]
pub fn list_drives() -> Vec<String> {
    vec!["/".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_dirs() {
        for d in ["node_modules", ".GIT", "__pycache__", ".svn", ".Hg", "$Recycle.Bin"] {
            assert!(skip_dir(d), "{d}");
        }
        assert!(!skip_dir("git") && !skip_dir("Documents"));
    }

    #[test]
    fn build_save_load_roundtrip() {
        let root = std::env::temp_dir().join(format!("bahth_idx_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("مستندات/فرعي")).unwrap();
        fs::write(root.join("مستندات/تقرير المبيعات.txt"), "x").unwrap();
        fs::write(root.join("مستندات/فرعي/Report.PDF"), "y").unwrap();

        let progress = ScanProgress::default();
        let idx = Index::build(&[root.to_str().unwrap().to_string()], &progress);
        assert_eq!(idx.entries.len(), 4);
        let e = idx.entries.iter().find(|e| e.name() == "Report.PDF").unwrap();
        assert_eq!(e.ext(), "pdf");
        assert_eq!(&*e.norm_name, "report.pdf");

        let cache = root.join("cache/index.bin");
        idx.save(&cache).unwrap();
        let loaded = Index::load(&cache).unwrap();
        assert_eq!(loaded.entries.len(), 4);
        assert!(loaded.entries.iter().any(|e| &*e.norm_name == "تقرير المبيعات.txt"));
        fs::remove_dir_all(&root).unwrap();
    }
}
