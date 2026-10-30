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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_changed_file_has_another_key() {
        assert_ne!(ThumbCache::key("C:\\a.jpg", 10, 1), ThumbCache::key("C:\\a.jpg", 10, 2));
        assert_ne!(ThumbCache::key("C:\\a.jpg", 10, 1), ThumbCache::key("C:\\a.jpg", 11, 1));
    }
}
