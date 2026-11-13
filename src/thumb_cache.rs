//! The memory cache behind the thumbnail view: which file's thumbnail sits
//! in which slot of the list's image list, bounded, least recently used
//! first out.
//!
//! Slots are numbered from 0 in the order they are handed out, so the caller
//! can tell a new slot (append to the image list) from a reused one (replace
//! it): a slot is new exactly when it equals the image list's count. Pinned
//! slots (the file-type icons) come from the same numbering and are never
//! evicted. Files without a thumbnail are remembered too, so they are not
//! asked for again; those entries hold no slot. A file that was cloud-only
//! when it was asked for (whose picture is only taken from the shell's own
//! cache, so as not to download it) is asked for again after
//! [`CLOUD_MISS_RETRY`]: OneDrive may have brought it to this PC since.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a thumbnail missed while the file's contents were online only
/// counts as missing. Its key (path, size, date) does not change when the
/// file is downloaded, so without this it would stay iconless until restart.
pub const CLOUD_MISS_RETRY: Duration = Duration::from_secs(30);

/// Whether a remembered miss still stands: `cloud_age` is how long ago it
/// was missed while the file was cloud-only, `None` for a file that was on
/// this PC (its miss stands until the file changes, which changes its key).
pub fn miss_stands(cloud_age: Option<Duration>) -> bool {
    cloud_age.is_none_or(|age| age < CLOUD_MISS_RETRY)
}

/// What the cache knows about one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cached {
    /// Its thumbnail is in this image-list slot.
    Image(i32),
    /// It has no thumbnail; show its file-type icon.
    NoThumb,
}

pub struct ThumbCache {
    max_images: usize,
    max_entries: usize,
    /// Key -> (what is cached, last use).
    map: HashMap<String, (Cached, u64)>,
    tick: u64,
    images: usize,
    next_slot: i32,
    /// Slots of thumbnails dropped from the cache, ready for reuse.
    free: Vec<i32>,
    /// When each miss of a then cloud-only file was remembered.
    cloud_misses: HashMap<String, Instant>,
}

impl ThumbCache {
    /// At most `max_images` thumbnails and `max_entries` files in all.
    pub fn new(max_images: usize, max_entries: usize) -> Self {
        ThumbCache {
            max_images: max_images.max(1),
            max_entries: max_entries.max(max_images.max(1)),
            map: HashMap::new(),
            tick: 0,
            images: 0,
            next_slot: 0,
            free: Vec::new(),
            cloud_misses: HashMap::new(),
        }
    }

    /// The key of a file: a changed file gets a new thumbnail.
    pub fn key(path: &str, size: u64, mtime: i64) -> String {
        format!("{mtime}|{size}|{path}")
    }

    /// Looks a file up, marking it as just used.
    pub fn get(&mut self, key: &str) -> Option<Cached> {
        self.get_at(key, Instant::now())
    }

    /// [`ThumbCache::get`] at time `now`: a cloud-only miss older than
    /// [`CLOUD_MISS_RETRY`] is forgotten, so the file is asked for again.
    fn get_at(&mut self, key: &str, now: Instant) -> Option<Cached> {
        if let Some(&at) = self.cloud_misses.get(key)
            && !miss_stands(Some(now.saturating_duration_since(at)))
        {
            self.cloud_misses.remove(key);
            self.map.remove(key);
            return None;
        }
        self.tick += 1;
        let tick = self.tick;
        let (c, used) = self.map.get_mut(key)?;
        *used = tick;
        Some(*c)
    }

    /// A slot that is never evicted.
    pub fn pin_slot(&mut self) -> i32 {
        let s = self.next_slot;
        self.next_slot += 1;
        s
    }

    /// Stores a thumbnail and returns the slot to put its picture in; when
    /// full, the least recently used thumbnail gives up its slot.
    pub fn insert_image(&mut self, key: String) -> i32 {
        self.tick += 1;
        if let Some((Cached::Image(s), used)) = self.map.get_mut(&key) {
            *used = self.tick;
            return *s;
        }
        self.map.remove(&key);
        self.cloud_misses.remove(&key);
        let slot = if let Some(s) = self.free.pop() {
            s
        } else if self.images < self.max_images {
            self.pin_slot()
        } else {
            self.evict(true).unwrap_or_else(|| self.pin_slot())
        };
        self.images += 1;
        self.map.insert(key, (Cached::Image(slot), self.tick));
        self.trim();
        slot
    }

    /// Remembers that a file has no thumbnail.
    pub fn insert_none(&mut self, key: String) {
        self.cloud_misses.remove(&key);
        self.insert_miss(key);
    }

    /// Remembers that a file that was cloud-only had no thumbnail in the
    /// shell's cache, for [`CLOUD_MISS_RETRY`].
    #[cfg_attr(not(windows), allow(dead_code))] // the Windows UI
    pub fn insert_cloud_none(&mut self, key: String) {
        self.insert_cloud_none_at(key, Instant::now());
    }

    fn insert_cloud_none_at(&mut self, key: String, now: Instant) {
        self.insert_miss(key.clone());
        if self.map.contains_key(&key) {
            self.cloud_misses.insert(key, now);
        }
    }

    fn insert_miss(&mut self, key: String) {
        self.tick += 1;
        if let Some((Cached::Image(s), _)) = self.map.remove(&key) {
            self.images -= 1;
            self.free.push(s);
        }
        self.map.insert(key, (Cached::NoThumb, self.tick));
        self.trim();
    }

    fn trim(&mut self) {
        while self.map.len() > self.max_entries {
            if let Some(s) = self.evict(false) {
                self.free.push(s);
            }
        }
    }

    /// Drops the least recently used entry (only thumbnails when
    /// `images_only`) and returns the slot it held.
    fn evict(&mut self, images_only: bool) -> Option<i32> {
        let key = self
            .map
            .iter()
            .filter(|(_, (c, _))| !images_only || matches!(c, Cached::Image(_)))
            .min_by_key(|(_, (_, used))| *used)
            .map(|(k, _)| k.clone())?;
        self.cloud_misses.remove(&key);
        match self.map.remove(&key)?.0 {
            Cached::Image(s) => {
                self.images -= 1;
                Some(s)
            }
            Cached::NoThumb => None,
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }
}

/// How many file types get an icon slot of their own in the thumbnail
/// view; the types seen after that share one generic file icon. Each slot
/// is a 96 px tile kept for the whole run, and each is made on the UI thread.
pub const MAX_TYPE_SLOTS: usize = 64;

/// The image-list slot of a file-type icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeSlot {
    /// Already drawn there.
    Known(i32),
    /// A new slot: draw the icon of this type into it ("" is the generic
    /// file icon, "/" the folder).
    New { slot: i32, ext: String },
}

/// Which file-type icon is in which pinned slot of the thumbnail view's
/// image list: at most [`MAX_TYPE_SLOTS`] types (folders always have their
/// own), then one shared slot for every other type.
pub struct TypeSlots {
    slots: HashMap<String, i32>,
    max: usize,
    generic: Option<i32>,
}

impl Default for TypeSlots {
    fn default() -> Self {
        TypeSlots::new(MAX_TYPE_SLOTS)
    }
}

impl TypeSlots {
    pub fn new(max: usize) -> Self {
        TypeSlots { slots: HashMap::new(), max, generic: None }
    }

    /// The slot for the icon of extension `ext` ("/" for folders), pinning
    /// a new one from `cache` when needed.
    pub fn slot(&mut self, ext: &str, cache: &mut ThumbCache) -> TypeSlot {
        if let Some(&s) = self.slots.get(ext) {
            return TypeSlot::Known(s);
        }
        if ext == "/" || self.slots.len() < self.max {
            let s = cache.pin_slot();
            self.slots.insert(ext.to_string(), s);
            return TypeSlot::New { slot: s, ext: ext.to_string() };
        }
        match self.generic {
            Some(s) => TypeSlot::Known(s),
            None => {
                let s = cache.pin_slot();
                self.generic = Some(s);
                TypeSlot::New { slot: s, ext: String::new() }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type icons: one slot per type up to the cap, then one shared generic
    /// slot; folders always their own; slots never collide with thumbnails.
    #[test]
    fn type_icon_slots_are_capped() {
        let mut cache = ThumbCache::new(4, 10);
        let mut t = TypeSlots::new(3);
        assert_eq!(t.slot("pdf", &mut cache), TypeSlot::New { slot: 0, ext: "pdf".into() });
        assert_eq!(t.slot("pdf", &mut cache), TypeSlot::Known(0));
        assert_eq!(t.slot("docx", &mut cache), TypeSlot::New { slot: 1, ext: "docx".into() });
        let thumb = cache.insert_image("a".into());
        assert_eq!(thumb, 2);
        assert_eq!(t.slot("xlsx", &mut cache), TypeSlot::New { slot: 3, ext: "xlsx".into() });
        // Past the cap: one generic slot, made once, shared by every other type.
        assert_eq!(t.slot("zip", &mut cache), TypeSlot::New { slot: 4, ext: String::new() });
        for ext in ["7z", "mp4", "zip", "", "x".repeat(300).as_str()] {
            assert_eq!(t.slot(ext, &mut cache), TypeSlot::Known(4), "{ext}");
        }
        // Folders are never shared.
        assert_eq!(t.slot("/", &mut cache), TypeSlot::New { slot: 5, ext: "/".into() });
        assert_eq!(t.slot("/", &mut cache), TypeSlot::Known(5));
        // Known types keep their slots; a thumbnail gets a fresh one.
        assert_eq!(t.slot("docx", &mut cache), TypeSlot::Known(1));
        assert_eq!(cache.insert_image("b".into()), 6);
        // A thousand types pin at most cap + generic + folder slots.
        let mut cache = ThumbCache::new(4, 10);
        let mut t = TypeSlots::default();
        let pinned: std::collections::HashSet<i32> = (0..1000)
            .map(|i| match t.slot(&format!("e{i}"), &mut cache) {
                TypeSlot::Known(s) | TypeSlot::New { slot: s, .. } => s,
            })
            .collect();
        assert_eq!(pinned.len(), MAX_TYPE_SLOTS + 1);
    }

    #[test]
    fn slots_are_handed_out_in_order_and_reused() {
        let mut c = ThumbCache::new(2, 10);
        assert_eq!(c.pin_slot(), 0);
        assert_eq!(c.insert_image("a".into()), 1);
        assert_eq!(c.insert_image("b".into()), 2);
        assert_eq!(c.get("a"), Some(Cached::Image(1)));
        // Full: "b" is the least recently used thumbnail.
        assert_eq!(c.insert_image("c".into()), 2);
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("c"), Some(Cached::Image(2)));
        assert_eq!(c.get("a"), Some(Cached::Image(1)));
        // The same file again keeps its slot.
        assert_eq!(c.insert_image("a".into()), 1);
    }

    #[test]
    fn files_without_thumbnail_are_remembered_and_bounded() {
        let mut c = ThumbCache::new(2, 3);
        c.insert_none("x".into());
        assert_eq!(c.get("x"), Some(Cached::NoThumb));
        let s = c.insert_image("a".into());
        c.insert_none("y".into());
        c.insert_none("z".into());
        assert_eq!(c.len(), 3);
        // "x" was the oldest; it went first.
        assert_eq!(c.get("x"), None);
        // Losing its thumbnail frees the slot for the next one.
        c.insert_none("a".into());
        assert_eq!(c.insert_image("b".into()), s);
    }

    /// A file missed while it was cloud-only is asked for again once OneDrive
    /// may have downloaded it; a local file's miss stands.
    #[test]
    fn cloud_only_misses_are_retried() {
        assert!(miss_stands(None));
        assert!(miss_stands(Some(Duration::ZERO)));
        assert!(!miss_stands(Some(CLOUD_MISS_RETRY)));
        let t0 = Instant::now();
        let later = t0 + CLOUD_MISS_RETRY;
        let mut c = ThumbCache::new(2, 4);
        c.insert_cloud_none_at("cloud".into(), t0);
        c.insert_none("local".into());
        assert_eq!(c.get_at("cloud", t0 + Duration::from_secs(1)), Some(Cached::NoThumb));
        assert_eq!(c.get_at("local", later), Some(Cached::NoThumb));
        assert_eq!(c.get_at("cloud", later), None);
        assert_eq!(c.len(), 1);
        // Asked again: now on this PC, it gets its picture.
        assert_eq!(c.insert_image("cloud".into()), 0);
        assert_eq!(c.get_at("cloud", later + CLOUD_MISS_RETRY), Some(Cached::Image(0)));
        // Missed again while local: it stands.
        c.insert_cloud_none_at("x".into(), t0);
        c.insert_none("x".into());
        assert_eq!(c.get_at("x", later), Some(Cached::NoThumb));
        // Evicted entries leave nothing behind.
        for i in 0..10 {
            c.insert_cloud_none_at(format!("c{i}"), t0);
        }
        assert!(c.cloud_misses.len() <= c.len());
    }

    #[test]
    fn trimming_frees_thumbnail_slots() {
        let mut c = ThumbCache::new(2, 2);
        let a = c.insert_image("a".into());
        c.insert_none("x".into());
        c.insert_none("y".into());
        assert_eq!(c.get("a"), None);
        assert_eq!(c.insert_image("b".into()), a);
    }

    /// Random lookups and inserts: never a panic, the bounds hold, and no
    /// two cached thumbnails share a slot (one would show the other's
    /// picture), nor a pinned slot.
    #[test]
    fn random_use_keeps_slots_unique_and_bounded() {
        let mut seed = 0x00DD_BA11_u64;
        let mut below = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for (max_images, max_entries) in [(1, 1), (2, 3), (5, 2), (8, 40), (0, 0)] {
            let mut c = ThumbCache::new(max_images, max_entries);
            let pinned: Vec<i32> = (0..3).map(|_| c.pin_slot()).collect();
            let mut handed_out = 0;
            for _ in 0..3000 {
                let key = ThumbCache::key(&format!("C:\\صور\\{}", below(30)), below(3) as u64, -(below(2) as i64));
                match below(3) {
                    0 => {
                        let s = c.insert_image(key.clone());
                        assert!(!pinned.contains(&s));
                        // A new slot is exactly the next one in the image list.
                        assert!(s as usize <= pinned.len() + handed_out);
                        handed_out = handed_out.max(s as usize + 1 - pinned.len());
                        assert_eq!(c.get(&key), Some(Cached::Image(s)));
                    }
                    1 => c.insert_none(key),
                    _ => {
                        let _ = c.get(&key);
                    }
                }
                let slots: Vec<i32> =
                    c.map.values().filter_map(|(v, _)| if let Cached::Image(s) = v { Some(*s) } else { None }).collect();
                let mut unique = slots.clone();
                unique.sort_unstable();
                unique.dedup();
                assert_eq!(unique.len(), slots.len(), "a slot is shared");
                assert_eq!(slots.len(), c.images);
                assert!(c.images <= c.max_images && c.len() <= c.max_entries);
                assert!(c.free.iter().all(|s| !slots.contains(s) && !pinned.contains(s)));
            }
        }
    }

    #[test]
    fn a_changed_file_has_another_key() {
        assert_ne!(ThumbCache::key("C:\\a.jpg", 10, 1), ThumbCache::key("C:\\a.jpg", 10, 2));
        assert_ne!(ThumbCache::key("C:\\a.jpg", 10, 1), ThumbCache::key("C:\\a.jpg", 11, 1));
    }
}
