//! Folder sizes from the index: how much space each folder takes, counting
//! everything inside it, and how many files it holds ("what uses my disk").
//!
//! Computed in one pass over the index entries, without touching the disk.
//! Each entry's parent is found by its path (everything before the last
//! separator); a parent that is not itself in the index, such as a drive
//! root `C:\`, becomes a *top* folder. Totals then flow from the deepest
//! entries up to their parents.
//!
//! The result is cached per index snapshot (`Arc<Index>`): when the index is
//! replaced, the next request computes the sizes for the new one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use crate::index::{Entry, Index};

/// Marks a parent that is a top folder rather than an entry.
const TOP: u32 = 1 << 31;
/// No parent (a path without any separator).
const NONE: u32 = u32::MAX;

/// A folder that holds indexed entries but is not an entry itself, usually
/// a drive root such as `C:\`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Top {
    pub path: String,
    pub size: u64,
    pub files: u64,
}

/// A place in the folder tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    /// Every drive and top folder together.
    All,
    /// A top folder, by its position in [`FolderSizes::tops`].
    Top(u32),
    /// An index entry, by its position in `Index::entries`.
    Entry(u32),
}

/// One item directly inside a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Child {
    pub node: Node,
    pub is_dir: bool,
    pub size: u64,
    pub files: u64,
}

/// Recursive size and file count of every entry of one index.
#[derive(Debug, Default)]
pub struct FolderSizes {
    /// A file's own size, or everything inside a folder.
    size: Vec<u64>,
    /// Files inside a folder, at any depth (1 for a file).
    files: Vec<u32>,
    /// Parent entry, `TOP | top` for a top folder, or `NONE`.
    parent: Vec<u32>,
    tops: Vec<Top>,
}

/// The folder that contains `path`, keeping the separator of a root
/// (`C:\x` → `C:\`, `/x` → `/`). `None` for a path without a separator.
pub fn parent_path(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(['\\', '/']);
    let i = trimmed.rfind(['\\', '/'])?;
    let parent = trimmed[..i].trim_end_matches(['\\', '/']);
    if parent.contains(['\\', '/']) {
        Some(parent)
    } else {
        // A root such as `C:` or `` keeps its separator: `C:\`, `/`.
        Some(&trimmed[..=parent.len()])
    }
}

fn depth(path: &str) -> usize {
    path.trim_end_matches(['\\', '/']).bytes().filter(|&b| b == b'\\' || b == b'/').count()
}

/// Same folder, ignoring ASCII case and a trailing separator.
fn same_folder(a: &str, b: &str) -> bool {
    fn trim(s: &str) -> &str {
        let t = s.trim_end_matches(['\\', '/']);
        if t.is_empty() { s } else { t }
    }
    trim(a).eq_ignore_ascii_case(trim(b))
}

impl FolderSizes {
    /// Computes every total in one pass: O(n) apart from hashing the paths.
    pub fn compute(entries: &[Entry]) -> FolderSizes {
        let n = entries.len();
        let dirs: HashMap<&str, u32> =
            entries.iter().enumerate().filter(|(_, e)| e.is_dir).map(|(i, e)| (&*e.path, i as u32)).collect();
        let mut top_ids: HashMap<&str, u32> = HashMap::new();
        let mut tops = Vec::new();
        let mut parent = Vec::with_capacity(n);
        let mut max_depth = 0;
        let mut depths = Vec::with_capacity(n);
        for e in entries {
            let p = match parent_path(&e.path) {
                None => NONE,
                Some(p) => match dirs.get(p) {
                    Some(&j) => j,
                    None => {
                        let t = *top_ids.entry(p).or_insert_with(|| {
                            tops.push(Top { path: p.to_string(), size: 0, files: 0 });
                            tops.len() as u32 - 1
                        });
                        TOP | t
                    }
                },
            };
            parent.push(p);
            let d = depth(&e.path);
            max_depth = max_depth.max(d);
            depths.push(d as u32);
        }
        drop(dirs);

        // Deepest entries first, so a folder's total is complete before it
        // is added to its own parent (a parent is always shallower).
        let mut start = vec![0usize; max_depth + 2];
        for &d in &depths {
            start[d as usize + 1] += 1;
        }
        for d in 1..start.len() {
            start[d] += start[d - 1];
        }
        let mut order = vec![0u32; n];
        for (i, &d) in depths.iter().enumerate() {
            order[start[d as usize]] = i as u32;
            start[d as usize] += 1;
        }
        drop(depths);

        let mut size: Vec<u64> = entries.iter().map(|e| if e.is_dir { 0 } else { e.size }).collect();
        let mut files: Vec<u32> = entries.iter().map(|e| u32::from(!e.is_dir)).collect();
        for &i in order.iter().rev() {
            let i = i as usize;
            let p = parent[i];
            if p == NONE {
            } else if p & TOP != 0 {
                let t = &mut tops[(p & !TOP) as usize];
                t.size += size[i];
                t.files += u64::from(files[i]);
            } else {
                size[p as usize] += size[i];
                files[p as usize] = files[p as usize].saturating_add(files[i]);
            }
        }
        FolderSizes { size, files, parent, tops }
    }

    /// Size of entry `i`: its own for a file, everything inside for a folder.
    pub fn total(&self, i: usize) -> u64 {
        self.size.get(i).copied().unwrap_or(0)
    }

    /// Files inside entry `i` at any depth (1 for a file).
    pub fn files(&self, i: usize) -> u64 {
        self.files.get(i).map_or(0, |&f| u64::from(f))
    }

    /// The top folders (drive roots), in the order they were met.
    #[cfg(test)]
    pub fn tops(&self) -> &[Top] {
        &self.tops
    }

    /// Size and file count of a node.
    pub fn node_total(&self, node: Node) -> (u64, u64) {
        match node {
            Node::All => self.tops.iter().fold((0, 0), |(s, f), t| (s + t.size, f + t.files)),
            Node::Top(t) => self.tops.get(t as usize).map_or((0, 0), |t| (t.size, t.files)),
            Node::Entry(i) => (self.total(i as usize), self.files(i as usize)),
        }
    }

    /// The folder at `path`: `""` is every drive, a drive root or another
    /// top folder, or a folder entry. Matches exactly first, then ignoring
    /// ASCII case. `None` for files and unknown paths.
    pub fn locate(&self, entries: &[Entry], path: &str) -> Option<Node> {
        if path.trim().is_empty() {
            return Some(Node::All);
        }
        let trimmed = path.trim_end_matches(['\\', '/']);
        if let Ok(i) = entries.binary_search_by(|e| (*e.path).cmp(trimmed))
            && entries[i].is_dir
        {
            return Some(Node::Entry(i as u32));
        }
        if let Some(t) = self.tops.iter().position(|t| same_folder(&t.path, path)) {
            return Some(Node::Top(t as u32));
        }
        entries.iter().position(|e| e.is_dir && same_folder(&e.path, path)).map(|i| Node::Entry(i as u32))
    }

    /// Path of a node; `""` for [`Node::All`].
    pub fn node_path<'a>(&'a self, entries: &'a [Entry], node: Node) -> &'a str {
        match node {
            Node::All => "",
            Node::Top(t) => self.tops.get(t as usize).map_or("", |t| &t.path),
            Node::Entry(i) => entries.get(i as usize).map_or("", |e| &e.path),
        }
    }

    /// The folder one level up, `None` from [`Node::All`].
    pub fn parent(&self, node: Node) -> Option<Node> {
        match node {
            Node::All => None,
            Node::Top(_) => Some(Node::All),
            Node::Entry(i) => match self.parent.get(i as usize).copied() {
                None | Some(NONE) => Some(Node::All),
                Some(p) if p & TOP != 0 => Some(Node::Top(p & !TOP)),
                Some(p) => Some(Node::Entry(p)),
            },
        }
    }

    /// What is directly inside `node`, largest first (ties in path order).
    pub fn children(&self, entries: &[Entry], node: Node) -> Vec<Child> {
        let mut out: Vec<Child> = match node {
            Node::All => (0..self.tops.len())
                .map(|t| Child { node: Node::Top(t as u32), is_dir: true, size: self.tops[t].size, files: self.tops[t].files })
                .collect(),
            Node::Top(_) | Node::Entry(_) => {
                let want = match node {
                    Node::Top(t) => TOP | t,
                    Node::Entry(i) => i,
                    Node::All => NONE,
                };
                self.parent
                    .iter()
                    .enumerate()
                    .filter(|&(_, &p)| p == want)
                    .map(|(i, _)| Child {
                        node: Node::Entry(i as u32),
                        is_dir: entries.get(i).is_some_and(|e| e.is_dir),
                        size: self.total(i),
                        files: self.files(i),
                    })
                    .collect()
            }
        };
        // Entries and tops are already in path order, so a stable sort keeps it for ties.
        out.sort_by_key(|c| std::cmp::Reverse(c.size));
        out
    }
}

/// Sorts result `hits` by size, with folders counted by everything inside
/// them, so big folders and big files mix. Ties keep index (path) order.
pub fn sort_hits_by_total(hits: &mut [u32], sizes: &FolderSizes, descending: bool) {
    use rayon::prelude::*;
    hits.par_sort_unstable_by(|&a, &b| {
        let o = sizes.total(a as usize).cmp(&sizes.total(b as usize));
        if descending { o.reverse() } else { o }.then(a.cmp(&b))
    });
}

/// The two most recent index snapshots and their sizes. Holding a `Weak`
/// keeps the snapshot's allocation from being reused while it is cached,
/// so comparing pointers is safe.
type Cache = Vec<(Weak<Index>, Arc<FolderSizes>)>;
static CACHE: Mutex<Cache> = Mutex::new(Vec::new());
/// One computation at a time: a second caller waits and reuses the result.
static COMPUTE: Mutex<()> = Mutex::new(());

/// The sizes for this index snapshot, if already computed. Never blocks on a
/// computation, so the UI thread can call it while drawing.
pub fn cached(index: &Arc<Index>) -> Option<Arc<FolderSizes>> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.iter().find(|(w, _)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(index))).map(|(_, s)| s.clone())
}

/// The sizes for this index snapshot, computing them if needed (call it off
/// the UI thread).
pub fn for_index(index: &Arc<Index>) -> Arc<FolderSizes> {
    if let Some(s) = cached(index) {
        return s;
    }
    let _turn = COMPUTE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = cached(index) {
        return s;
    }
    let sizes = Arc::new(FolderSizes::compute(&index.entries));
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(w, _)| w.strong_count() > 0);
    if cache.len() >= 2 {
        cache.remove(0);
    }
    cache.push((Arc::downgrade(index), sizes.clone()));
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, size: u64, is_dir: bool) -> Entry {
        Entry { path: path.into(), size, mtime: 0, is_dir, norm_name: Box::default() }
    }

    /// Entries sorted by path, as the index keeps them.
    fn entries(list: &[(&str, u64, bool)]) -> Vec<Entry> {
        let mut v: Vec<Entry> = list.iter().map(|&(p, s, d)| entry(p, s, d)).collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    }

    fn at(v: &[Entry], path: &str) -> usize {
        v.iter().position(|e| &*e.path == path).unwrap()
    }

    #[test]
    fn parent_paths() {
        assert_eq!(parent_path(r"C:\a\b"), Some(r"C:\a"));
        assert_eq!(parent_path(r"C:\a"), Some(r"C:\"));
        assert_eq!(parent_path(r"C:\a\"), Some(r"C:\"));
        assert_eq!(parent_path(r"\\srv\share\x"), Some(r"\\srv\share"));
        assert_eq!(parent_path("/home/x"), Some("/home"));
        assert_eq!(parent_path("/home"), Some("/"));
        assert_eq!(parent_path("C:"), None);
        assert_eq!(parent_path(r"C:\مستندات\تقرير.docx"), Some(r"C:\مستندات"));
    }

    #[test]
    fn totals_add_up_through_every_level() {
        let v = entries(&[
            (r"C:\a", 0, true),
            (r"C:\a\x.txt", 10, false),
            (r"C:\a\b", 0, true),
            (r"C:\a\b\y.bin", 100, false),
            (r"C:\a\b\c", 0, true),
            (r"C:\a\b\c\z", 1000, false),
            (r"C:\a b", 0, true),
            (r"C:\a b\w", 5, false),
            (r"C:\top.txt", 7, false),
            (r"C:\empty", 0, true),
        ]);
        let s = FolderSizes::compute(&v);
        assert_eq!(s.total(at(&v, r"C:\a")), 1110);
        assert_eq!(s.files(at(&v, r"C:\a")), 3);
        assert_eq!(s.total(at(&v, r"C:\a\b")), 1100);
        assert_eq!(s.total(at(&v, r"C:\a\b\c")), 1000);
        // A sibling whose name starts the same is not inside it.
        assert_eq!(s.total(at(&v, r"C:\a b")), 5);
        assert_eq!(s.total(at(&v, r"C:\empty")), 0);
        assert_eq!(s.files(at(&v, r"C:\empty")), 0);
        assert_eq!(s.total(at(&v, r"C:\top.txt")), 7);
        assert_eq!(s.files(at(&v, r"C:\top.txt")), 1);
        assert_eq!(s.tops(), &[Top { path: r"C:\".into(), size: 1122, files: 5 }]);
        assert_eq!(s.node_total(Node::All), (1122, 5));
    }

    #[test]
    fn arabic_paths() {
        let v = entries(&[
            (r"D:\مستندات", 0, true),
            (r"D:\مستندات\تقرير.docx", 2048, false),
            (r"D:\مستندات\صور", 0, true),
            (r"D:\مستندات\صور\رحلة.jpg", 4096, false),
            (r"D:\مستندات قديمة", 0, true),
            (r"D:\مستندات قديمة\ملف.txt", 1, false),
        ]);
        let s = FolderSizes::compute(&v);
        assert_eq!(s.total(at(&v, r"D:\مستندات")), 6144);
        assert_eq!(s.files(at(&v, r"D:\مستندات")), 2);
        assert_eq!(s.total(at(&v, r"D:\مستندات\صور")), 4096);
        assert_eq!(s.total(at(&v, r"D:\مستندات قديمة")), 1);
        let node = s.locate(&v, r"D:\مستندات\").unwrap();
        assert_eq!(node, Node::Entry(at(&v, r"D:\مستندات") as u32));
        let kids = s.children(&v, node);
        let names: Vec<&str> = kids.iter().map(|c| s.node_path(&v, c.node)).collect();
        assert_eq!(names, [r"D:\مستندات\صور", r"D:\مستندات\تقرير.docx"]);
        assert!(kids[0].is_dir && !kids[1].is_dir);
    }

    #[test]
    fn many_drives_and_navigation() {
        let mut list = Vec::new();
        let drives: Vec<String> = ('C'..='Z').map(|d| format!(r"{d}:\data")).collect();
        let files: Vec<String> = drives.iter().map(|d| format!(r"{d}\f")).collect();
        for (i, (d, f)) in drives.iter().zip(&files).enumerate() {
            list.push((d.as_str(), 0, true));
            list.push((f.as_str(), (i as u64 + 1) * 10, false));
        }
        let v = entries(&list);
        let s = FolderSizes::compute(&v);
        assert_eq!(s.tops().len(), 24);
        assert_eq!(s.node_total(Node::All).1, 24);
        let all = s.children(&v, Node::All);
        assert_eq!(all.len(), 24);
        // Largest drive first: Z holds 240 bytes.
        assert_eq!(s.node_path(&v, all[0].node), r"Z:\");
        assert_eq!(all[0].size, 240);
        let z = s.locate(&v, "z:").unwrap();
        assert_eq!(z, all[0].node);
        let kids = s.children(&v, z);
        assert_eq!(kids.len(), 1);
        let data = kids[0].node;
        assert_eq!(s.node_path(&v, data), r"Z:\data");
        assert_eq!(s.parent(data), Some(z));
        assert_eq!(s.parent(z), Some(Node::All));
        assert_eq!(s.parent(Node::All), None);
        assert_eq!(s.locate(&v, r"Z:\data\f"), None, "files are not folders");
        assert_eq!(s.locate(&v, r"z:\DATA"), Some(data), "case is ignored as a fallback");
        assert_eq!(s.locate(&v, ""), Some(Node::All));
    }

    #[test]
    fn unc_and_unix_roots() {
        let v = entries(&[(r"\\srv\share\a", 0, true), (r"\\srv\share\a\b", 3, false), ("/home", 0, true), ("/home/x", 4, false)]);
        let s = FolderSizes::compute(&v);
        let tops: Vec<&str> = s.tops().iter().map(|t| t.path.as_str()).collect();
        assert!(tops.contains(&"/") && tops.contains(&r"\\srv\share"));
        assert_eq!(s.total(at(&v, "/home")), 4);
        assert_eq!(s.total(at(&v, r"\\srv\share\a")), 3);
    }

    #[test]
    fn sorting_hits_by_total() {
        let v = entries(&[(r"C:\big", 0, true), (r"C:\big\f", 500, false), (r"C:\mid.bin", 50, false), (r"C:\small", 0, true)]);
        let s = FolderSizes::compute(&v);
        let mut hits: Vec<u32> = (0..v.len() as u32).collect();
        sort_hits_by_total(&mut hits, &s, true);
        let order: Vec<&str> = hits.iter().map(|&i| &*v[i as usize].path).collect();
        assert_eq!(order, [r"C:\big", r"C:\big\f", r"C:\mid.bin", r"C:\small"]);
        sort_hits_by_total(&mut hits, &s, false);
        assert_eq!(&*v[hits[0] as usize].path, r"C:\small");
    }

    #[test]
    fn cache_is_keyed_on_the_snapshot() {
        let a = Arc::new(Index { entries: entries(&[(r"C:\x", 0, true), (r"C:\x\y", 9, false)]), ..Default::default() });
        let b = Arc::new(Index { entries: entries(&[(r"C:\x", 0, true), (r"C:\x\y", 1, false)]), ..Default::default() });
        let sa = for_index(&a);
        assert_eq!(sa.total(0), 9);
        assert!(Arc::ptr_eq(&cached(&a).unwrap(), &sa));
        let sb = for_index(&b);
        assert_eq!(sb.total(0), 1, "a new snapshot gets its own sizes");
        assert_eq!(cached(&a).unwrap().total(0), 9);
    }

    #[test]
    fn large_index_is_linear_enough() {
        // 200k entries in 2k folders: must finish quickly even in debug builds.
        let mut v = Vec::new();
        for d in 0..2000 {
            v.push(entry(&format!(r"C:\d{d:04}"), 0, true));
            for f in 0..99 {
                v.push(entry(&format!(r"C:\d{d:04}\f{f:02}"), 1, false));
            }
        }
        v.sort_by(|a, b| a.path.cmp(&b.path));
        let t = std::time::Instant::now();
        let s = FolderSizes::compute(&v);
        assert!(t.elapsed().as_secs() < 10);
        assert_eq!(s.tops()[0].size, 2000 * 99);
        assert_eq!(s.total(at(&v, r"C:\d0007")), 99);
    }
}
