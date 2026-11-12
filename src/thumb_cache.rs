//! The memory cache behind the thumbnail view: which file's thumbnail sits
//! in which slot of the list's image list, bounded, least recently used
//! first out.
//!
//! Slots are numbered from 0 in the order they are handed out, so the caller
//! can tell a new slot (append to the image list) from a reused one (replace
//! it): a slot is new exactly when it equals the image list's count. Pinned
//! slots (the file-type icons) come from the same numbering and are never
//! evicted. Files without a thumbnail are remembered too, so they are not
//! asked for again; those entries hold no slot.

use std::collections::HashMap;

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
        }
    }

    /// The key of a file: a changed file gets a new thumbnail.
    pub fn key(path: &str, size: u64, mtime: i64) -> String {
        format!("{mtime}|{size}|{path}")
    }

    /// Looks a file up, marking it as just used.
    pub fn get(&mut self, key: &str) -> Option<Cached> {
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
