//! Temporary files and folders for the tests, removed when their guard is
//! dropped. A failing test unwinds through the guard, so nothing it wrote
//! stays behind in the temp folder (some write hundreds of MB).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A path in the temp folder; dropping it removes `owned` (a file, or a
/// folder with everything in it), which is `path` or a folder holding it.
#[derive(Debug)]
pub struct TempPath {
    path: PathBuf,
    owned: PathBuf,
}

impl TempPath {
    /// Removes `path` (a file or a folder) on drop. Nothing is created.
    pub fn at(path: impl Into<PathBuf>) -> TempPath {
        let path = path.into();
        TempPath { owned: path.clone(), path }
    }

    /// `<temp>/<name>_<pid>/<file>` with `n` a number new to this process,
    /// so tests running at once never share it: the folder is created (the
    /// file is not) and dropping the guard removes the folder.
    pub fn in_own_dir(name: &str, file: &str) -> TempPath {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{name}_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempPath { path: dir.join(file), owned: dir }
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        match std::fs::symlink_metadata(&self.owned) {
            Ok(m) if m.is_dir() => {
                let _ = std::fs::remove_dir_all(&self.owned);
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&self.owned);
            }
            Err(_) => {}
        }
    }
}

impl std::ops::Deref for TempPath {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_on_drop_even_after_a_panic() {
        let file = std::env::temp_dir().join(format!("bahth_test_temp_{}.txt", std::process::id()));
        let r = std::panic::catch_unwind(|| {
            let g = TempPath::at(file.clone());
            std::fs::write(&g, "x").unwrap();
            panic!("a failing test");
        });
        assert!(r.is_err());
        assert!(!file.exists());

        let (a, b) = (TempPath::in_own_dir("bahth_test_temp", "a.txt"), TempPath::in_own_dir("bahth_test_temp", "a.txt"));
        assert_ne!(a.parent(), b.parent());
        std::fs::write(&a, "x").unwrap();
        let dir = a.parent().unwrap().to_path_buf();
        drop(a);
        assert!(!dir.exists());
        // A path never made: nothing to remove.
        drop(TempPath::at(std::env::temp_dir().join(format!("bahth_test_temp_none_{}", std::process::id()))));
    }
}
