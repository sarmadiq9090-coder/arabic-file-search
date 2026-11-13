//! The word index of the content store: which documents hold which words,
//! so a search inside documents reads only the documents that can match
//! instead of every stored text (on a big PC: 290k documents, a 1 GB
//! `content.dat`, a minute and more per search).
//!
//! It is only a filter. A query is turned into a [`Plan`] (by
//! `search::word_plan`): for each term, the shapes its alternatives can take
//! in a document's words (inside a word, at its start, at its end, the whole
//! word, after an article…). The index gives the documents whose words have
//! those shapes, and each of them is then read and matched by the very same
//! matcher as before, so the results are the ones a full scan finds. A
//! document the index does not hold for its current text (not indexed yet,
//! changed since, the index missing or damaged) is read and matched as
//! before. Words are those of the normalised text ([`doc_words`]), the text
//! the matcher reads.
//!
//! On disk, next to `content.dat` (one folder, see `content_cache`):
//! - `words-<serial>.seg`: immutable segments. Each holds a sorted vocabulary
//!   in blocks (`\nword\nword\n…\n`, so one Aho-Corasick pass over a block
//!   finds the words containing, starting or ending with a query piece, or
//!   equal to it), the length of each word's postings, and the postings
//!   (document ids, delta-coded). A trailer and a block table with
//!   checksums; written to a `.tmp`, synced and renamed.
//! - `words.man`: the manifest (bincode, checksum): the segments, and for
//!   every document in them its path key, id, size, modification time and
//!   the checksum of its stored text. Written via a temp file and a rename.
//!
//! Opening compares the manifest with the content store: a document counts
//! as indexed only when its size, time and text checksum are the store's;
//! everything else is indexed again from the stored text by the warm-up
//! (`ContentCache::index_stored`), without reading the file. A damaged
//! manifest or segment starts the index afresh. So a crash at any point can
//! leave only documents that are not indexed yet (read in full by the next
//! searches), never a wrong answer.
//!
//! In memory: a map from path key to document id (about 17 bytes a
//! document), the block table of each segment (about 50 bytes per 32 KB of
//! vocabulary), and the documents indexed since the last segment, their
//! word lists, until [`PENDING_DOCS`] or [`PENDING_BYTES`]. A changed or
//! deleted document leaves its old postings in its segment (a tombstone:
//! its id is no longer in the map); merges drop them.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::hash::{BuildHasherDefault, Hasher};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use aho_corasick::AhoCorasick;
use serde::{Deserialize, Serialize};

use crate::content_cache::{fnv, read_at};

/// Bumped whenever the on-disk format of the word index changes (or what a
/// word is): the index is then built again from the stored texts.
const VERSION: u32 = 1;
const SEG_MAGIC: [u8; 8] = *b"BHTHWSG\0";
const MANIFEST: &str = "words.man";
const SEG_PREFIX: &str = "words-";
const SEG_EXT: &str = ".seg";

/// Words longer than this (in bytes, normalised) are not indexed: the
/// document is marked instead, and such documents are read by every search.
pub const MAX_WORD: usize = 128;
/// Documents indexed since the last segment, at most, before one is written.
pub const PENDING_DOCS: usize = 2048;
/// Their word lists' bytes, at most.
pub const PENDING_BYTES: usize = 4 << 20;
/// A vocabulary block holds about this much text.
const BLOCK_VOCAB: usize = 32 << 10;
/// A block ends once its postings pass this.
const BLOCK_POSTINGS: usize = 256 << 10;
/// More segments than this are merged even while the warm-up runs.
const MAX_SEGMENTS: usize = 16;
/// A query piece found in more words than this, or with more postings
/// than [`PIECE_MAX_BYTES`], narrows nothing (it would list nearly every
/// document anyway): the other pieces of the query still do.
const PIECE_MAX_WORDS: usize = 200_000;
const PIECE_MAX_BYTES: usize = 64 << 20;

// ---------------------------------------------------------------------------
// Keys, varints, bitsets
// ---------------------------------------------------------------------------

/// The key of a document's path in the index: FNV-1a with a mixing step
/// (64 bits; two of 300k paths share one with odds of about 1 in 400
/// million, and then one of them is read by every search or missed by the
/// filter until it changes).
pub fn key(path: &str) -> u64 {
    let mut h = fnv(path.as_bytes());
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// The keys are hashes already.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

type Ids = HashMap<u64, u32, BuildHasherDefault<KeyHasher>>;

fn fnv32(bytes: &[u8]) -> u32 {
    let h = fnv(bytes);
    (h ^ (h >> 32)) as u32
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*at)?;
        *at += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(v);
        }
    }
    None
}

/// Appends the ascending `ids` (all `>= min`) to `out`: each as its distance
/// from the one before plus one (from `min` for the first), so no value is 0.
fn encode_ids(out: &mut Vec<u8>, ids: impl IntoIterator<Item = u32>, min: u32) {
    let mut next = u64::from(min);
    for id in ids {
        put_varint(out, u64::from(id) + 1 - next);
        next = u64::from(id) + 1;
    }
}

/// Calls `f` with every id of `encode_ids` output. False when the bytes are
/// damaged: a 0, ids not rising, or one outside `[min, end)`.
fn decode_ids(b: &[u8], min: u32, end: u32, mut f: impl FnMut(u32)) -> bool {
    let (mut at, mut next) = (0, u64::from(min));
    while at < b.len() {
        let Some(d) = get_varint(b, &mut at) else { return false };
        if d == 0 {
            return false;
        }
        let id = next + d - 1;
        if id >= u64::from(end) {
            return false;
        }
        f(id as u32);
        next = id + 1;
    }
    true
}

fn bitset(end: u32) -> Vec<u64> {
    vec![0; (end as usize).div_ceil(64)]
}

fn set_bit(bits: &mut [u64], id: u32) {
    if let Some(w) = bits.get_mut(id as usize / 64) {
        *w |= 1 << (id % 64);
    }
}

/// Whether `id` is set in `bits`.
pub fn has_bit(bits: &[u64], id: u32) -> bool {
    bits.get(id as usize / 64).is_some_and(|w| w & (1 << (id % 64)) != 0)
}

/// The ids in `ids` (the values) below `end`, as a bitset.
fn live_bits(ids: &Ids, end: u32) -> Vec<u64> {
    let mut bits = bitset(end);
    for &id in ids.values() {
        set_bit(&mut bits, id);
    }
    bits
}

fn count_bits(bits: &[u64], min: u32, end: u32) -> u32 {
    (min..end).filter(|&id| has_bit(bits, id)).count() as u32
}

// ---------------------------------------------------------------------------
// The words of a document
// ---------------------------------------------------------------------------

/// The distinct words of a document's normalised text, as `\nword\n…\n`
/// (empty when it has none), and whether one was longer than [`MAX_WORD`].
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Words {
    list: String,
    long: bool,
}

impl Words {
    /// The words of a document with no text.
    pub fn none() -> Words {
        Words::default()
    }

    /// The words, sorted (for the tests).
    #[cfg(test)]
    pub fn words(&self) -> Vec<&str> {
        let mut w: Vec<&str> = self.list.split('\n').filter(|w| !w.is_empty()).collect();
        w.sort_unstable();
        w
    }
}

/// Words are collected this many at a time, then sorted and deduplicated:
/// a typical document needs no set (and no allocation per word) at all.
const WORD_CHUNK: usize = 8192;

struct Tokens {
    /// The words of the current chunk, end to end, and where each ends.
    chunk: String,
    ends: Vec<usize>,
    /// The distinct words of the chunks before (big documents only).
    seen: HashSet<Box<str>>,
    long: bool,
    word: String,
}

impl Tokens {
    fn end(&mut self) {
        if self.word.is_empty() {
            return;
        }
        if self.word.len() > MAX_WORD {
            self.long = true;
        } else {
            self.chunk.push_str(&self.word);
            self.ends.push(self.chunk.len());
            if self.ends.len() >= WORD_CHUNK {
                self.spill();
            }
        }
        self.word.clear();
    }

    /// The chunk's distinct words (in no particular order: hashing 300
    /// words is several times faster than sorting them).
    fn distinct(&self) -> HashSet<&str> {
        let mut start = 0;
        let mut words = HashSet::with_capacity(self.ends.len());
        for &e in &self.ends {
            words.insert(&self.chunk[start..e]);
            start = e;
        }
        words
    }

    fn spill(&mut self) {
        let mut seen = std::mem::take(&mut self.seen);
        for w in self.distinct() {
            if !seen.contains(w) {
                seen.insert(w.into());
            }
        }
        self.seen = seen;
        self.chunk.clear();
        self.ends.clear();
    }

    fn finish(mut self) -> Words {
        self.end();
        let mut list = String::new();
        let mut push = |w: &str| {
            if list.is_empty() {
                list.push('\n');
            }
            list.push_str(w);
            list.push('\n');
        };
        if self.seen.is_empty() {
            self.distinct().into_iter().for_each(&mut push);
        } else {
            self.spill();
            self.seen.iter().for_each(|w| push(w));
        }
        Words { list, long: self.long }
    }
}

/// The words of `text` as the matcher sees them: `normalize(text)` split
/// into runs of letters and digits (`search::split_words`), each once.
/// Normalised one character at a time, so the whole normalised
/// text is never held.
pub fn doc_words(text: &str) -> Words {
    let mut t = Tokens { chunk: String::new(), ends: Vec::new(), seen: HashSet::new(), long: false, word: String::new() };
    let mut buf = String::new();
    for c in text.chars() {
        if c.is_ascii() {
            // normalize: ASCII is lower-cased, and \u{1} \u{2} dropped.
            if c.is_ascii_alphanumeric() {
                t.word.push(c.to_ascii_lowercase());
            } else if c != crate::normalize::FORM_SEP && c != crate::normalize::SPELLING_SEP {
                t.end();
            }
            continue;
        }
        // Arabic letters that normalise to themselves (not the alef and
        // hamza forms, ة, ى, nor the tatweel): letters, so part of a word.
        if ('\u{0621}'..='\u{064A}').contains(&c) && !matches!(c, 'أ' | 'إ' | 'آ' | 'ة' | 'ى' | 'ئ' | 'ؤ' | '\u{0640}') {
            t.word.push(c);
            continue;
        }
        buf.clear();
        crate::normalize::push_normalized(&mut buf, c);
        for n in buf.chars() {
            if crate::search::is_word_char(n) {
                t.word.push(n);
            } else {
                t.end();
            }
        }
    }
    t.finish()
}

// ---------------------------------------------------------------------------
// Query plans
// ---------------------------------------------------------------------------

/// What a document's words must hold for a query to possibly match it: every
/// condition (a term) holds when one of its alternatives does, and an
/// alternative holds when each of its pieces does. A piece holds when one of
/// the document's words, written `\nword\n`, contains one of its patterns
/// (`abc` inside a word, `\nabc` at its start, `abc\n` at its end, `\nabc\n`
/// the whole word). A document with a word longer than [`MAX_WORD`] holds
/// every piece.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Plan {
    /// Each piece's patterns.
    pub pieces: Vec<Vec<Vec<u8>>>,
    /// Conditions → alternatives → piece numbers.
    pub conds: Vec<Vec<Vec<usize>>>,
}

impl Plan {
    /// The number of the piece with these patterns, added if new.
    pub fn piece(&mut self, patterns: Vec<Vec<u8>>) -> usize {
        match self.pieces.iter().position(|p| *p == patterns) {
            Some(i) => i,
            None => {
                self.pieces.push(patterns);
                self.pieces.len() - 1
            }
        }
    }
}

/// What [`Index::candidates`] found.
pub enum Found {
    /// The documents (ids below `end`) that may match.
    Ids { bits: Vec<u64>, end: u32 },
    /// Every condition is too broad to narrow anything.
    Everything,
    /// The index can't answer now (damaged, or writing failed).
    Unavailable,
}

/// Per piece: the documents found so far, or `None` once it narrows nothing.
struct Acc {
    bits: Vec<Option<Vec<u64>>>,
    words: Vec<usize>,
    bytes: Vec<usize>,
}

impl Acc {
    fn new(pieces: usize, end: u32) -> Acc {
        Acc { bits: (0..pieces).map(|_| Some(bitset(end))).collect(), words: vec![0; pieces], bytes: vec![0; pieces] }
    }

    fn wants(&self, piece: usize) -> bool {
        self.bits[piece].is_some()
    }

    fn set(&mut self, piece: usize, id: u32) {
        if let Some(b) = &mut self.bits[piece] {
            set_bit(b, id);
        }
    }

    fn set_all(&mut self, id: u32) {
        for b in self.bits.iter_mut().flatten() {
            set_bit(b, id);
        }
    }

    /// Counts one more word of `bytes` postings for `piece`; false (and the
    /// piece narrows nothing from now on) past the limits.
    fn charge(&mut self, piece: usize, bytes: usize) -> bool {
        self.words[piece] += 1;
        self.bytes[piece] += bytes;
        if self.words[piece] > PIECE_MAX_WORDS || self.bytes[piece] > PIECE_MAX_BYTES {
            self.bits[piece] = None;
        }
        self.bits[piece].is_some()
    }

    /// The plan's answer: `None` when no condition narrows anything.
    fn evaluate(self, plan: &Plan, end: u32) -> Option<Vec<u64>> {
        let mut result: Option<Vec<u64>> = None;
        'conds: for cond in &plan.conds {
            // Documents holding one of the alternatives.
            let mut any = bitset(end);
            for alt in cond {
                // Documents holding all its pieces (None: any document).
                let mut all: Option<Vec<u64>> = None;
                for b in alt.iter().filter_map(|&p| self.bits[p].as_ref()) {
                    match &mut all {
                        None => all = Some(b.clone()),
                        Some(a) => a.iter_mut().zip(b).for_each(|(x, y)| *x &= y),
                    }
                }
                // An alternative that narrows nothing: neither does the condition.
                let Some(all) = all else { continue 'conds };
                any.iter_mut().zip(&all).for_each(|(x, y)| *x |= y);
            }
            match &mut result {
                None => result = Some(any),
                Some(r) => r.iter_mut().zip(&any).for_each(|(x, y)| *x &= y),
            }
        }
        result
    }
}

/// The plan's distinct patterns, and for each the pieces it belongs to.
struct Matcher {
    ac: AhoCorasick,
    owners: Vec<Vec<usize>>,
}

impl Matcher {
    fn new(plan: &Plan) -> Option<Matcher> {
        let mut pats: Vec<&[u8]> = Vec::new();
        let mut owners: Vec<Vec<usize>> = Vec::new();
        for (p, list) in plan.pieces.iter().enumerate() {
            for pat in list {
                match pats.iter().position(|x| *x == pat.as_slice()) {
                    Some(i) => owners[i].push(p),
                    None => {
                        pats.push(pat);
                        owners.push(vec![p]);
                    }
                }
            }
        }
        if pats.is_empty() || pats.iter().any(|p| p.is_empty()) {
            return None;
        }
        Some(Matcher { ac: AhoCorasick::new(&pats).ok()?, owners })
    }

    /// The pieces found in a document's word list.
    fn doc(&self, list: &[u8], mut f: impl FnMut(usize)) {
        for m in self.ac.find_overlapping_iter(list) {
            self.owners[m.pattern().as_usize()].iter().for_each(|&p| f(p));
        }
    }
}

// ---------------------------------------------------------------------------
// Segments
// ---------------------------------------------------------------------------

/// Trailer: magic, version, min, end, docs, blocks, table offset and length,
/// long-postings offset and length, table checksum, trailer checksum.
const TRAILER: usize = 8 + 4 * 5 + 8 + 4 + 8 + 4 + 8 + 8;

/// Each word's postings, as a byte range of its block's postings.
type Ranges = Vec<(u32, u32)>;

struct Block {
    off: u64,
    vocab: u32,
    lens: u32,
    post: u32,
    words: u32,
    sum: u32,
}

struct Segment {
    serial: u64,
    path: PathBuf,
    file: File,
    /// Ids `[min, end)`.
    min: u32,
    end: u32,
    /// Documents it was written with (dead ones included).
    docs: u32,
    bytes: u64,
    blocks: Vec<Block>,
    long_off: u64,
    long_len: u32,
}

fn seg_path(dir: &Path, serial: u64) -> PathBuf {
    dir.join(format!("{SEG_PREFIX}{serial:016x}{SEG_EXT}"))
}

fn corrupt() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "word index damaged")
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or_default())
}

fn le_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default())
}

impl Segment {
    fn open(dir: &Path, serial: u64) -> io::Result<Segment> {
        let path = seg_path(dir, serial);
        let file = File::open(&path)?;
        let bytes = file.metadata()?.len();
        if bytes < TRAILER as u64 {
            return Err(corrupt());
        }
        let mut t = [0u8; TRAILER];
        read_at(&file, &mut t, bytes - TRAILER as u64)?;
        if t[..8] != SEG_MAGIC || le_u32(&t, 8) != VERSION || fnv(&t[..TRAILER - 8]) != le_u64(&t, TRAILER - 8) {
            return Err(corrupt());
        }
        let (min, end, docs, n) = (le_u32(&t, 12), le_u32(&t, 16), le_u32(&t, 20), le_u32(&t, 24) as usize);
        let (table_off, table_len) = (le_u64(&t, 28), le_u32(&t, 36) as u64);
        let (long_off, long_len) = (le_u64(&t, 40), le_u32(&t, 48));
        let data_end = bytes - TRAILER as u64;
        if min > end || table_off.checked_add(table_len).is_none_or(|e| e > data_end) || long_off.checked_add(u64::from(long_len)).is_none_or(|e| e > table_off) {
            return Err(corrupt());
        }
        let mut table = vec![0u8; table_len as usize];
        read_at(&file, &mut table, table_off)?;
        if fnv(&table) != le_u64(&t, 52) {
            return Err(corrupt());
        }
        let mut blocks = Vec::with_capacity(n.min(table.len() / 30));
        let mut at = 0;
        for _ in 0..n {
            let first_len = usize::from(u16::from_le_bytes(table.get(at..at + 2).ok_or_else(corrupt)?.try_into().unwrap_or_default()));
            let rec = table.get(at + 2..at + 2 + first_len + 28).ok_or_else(corrupt)?;
            let f = first_len;
            // `rec[..f]`, the block's first word, is not needed in memory.
            let b = Block {
                off: le_u64(rec, f),
                vocab: le_u32(rec, f + 8),
                lens: le_u32(rec, f + 12),
                post: le_u32(rec, f + 16),
                words: le_u32(rec, f + 20),
                sum: le_u32(rec, f + 24),
            };
            let size = u64::from(b.vocab) + u64::from(b.lens) + u64::from(b.post);
            if b.off.checked_add(size).is_none_or(|e| e > long_off) {
                return Err(corrupt());
            }
            blocks.push(b);
            at += 2 + first_len + 28;
        }
        if at != table.len() {
            return Err(corrupt());
        }
        Ok(Segment { serial, path, file, min, end, docs, bytes, blocks, long_off, long_len })
    }

    /// Reads block `b`: its vocabulary and word lengths, and with `postings`
    /// its postings too. Returns the bytes and each word's postings range
    /// (relative to the postings).
    fn read_block(&self, b: &Block, postings: bool) -> io::Result<(Vec<u8>, Ranges)> {
        let head = b.vocab as usize + b.lens as usize;
        let mut buf = vec![0u8; head + if postings { b.post as usize } else { 0 }];
        read_at(&self.file, &mut buf, b.off)?;
        if fnv32(&buf[..head]) != b.sum || buf.first() != Some(&b'\n') || buf[..b.vocab as usize].last() != Some(&b'\n') {
            return Err(corrupt());
        }
        let lens = &buf[b.vocab as usize..head];
        let mut ranges = Vec::with_capacity(b.words as usize);
        let (mut at, mut pos) = (0, 0u64);
        for _ in 0..b.words {
            let len = get_varint(lens, &mut at).ok_or_else(corrupt)?;
            let end = pos + len;
            if end > u64::from(b.post) {
                return Err(corrupt());
            }
            ranges.push((pos as u32, end as u32));
            pos = end;
        }
        if at != lens.len() || pos != u64::from(b.post) {
            return Err(corrupt());
        }
        Ok((buf, ranges))
    }

    fn read_long(&self) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; self.long_len as usize];
        read_at(&self.file, &mut buf, self.long_off)?;
        Ok(buf)
    }

    /// Adds what this segment holds for the plan's pieces to `acc`.
    fn lookup(&self, m: &Matcher, acc: &mut Acc) -> io::Result<()> {
        let long = self.read_long()?;
        if !decode_ids(&long, self.min, self.end, |id| acc.set_all(id)) {
            return Err(corrupt());
        }
        let mut hits: Vec<(u32, usize)> = Vec::new();
        for b in &self.blocks {
            let (buf, ranges) = self.read_block(b, false)?;
            let vocab = &buf[..b.vocab as usize];
            hits.clear();
            // Newlines in vocab[..pos]: the word at p is number count - 1.
            let (mut pos, mut count) = (0usize, 0usize);
            for hit in m.ac.find_overlapping_iter(vocab) {
                let p = hit.start() + usize::from(vocab[hit.start()] == b'\n');
                if p >= pos {
                    count += memchr::memchr_iter(b'\n', &vocab[pos..p]).count();
                } else {
                    count -= memchr::memchr_iter(b'\n', &vocab[p..pos]).count();
                }
                pos = p;
                let w = count.checked_sub(1).ok_or_else(corrupt)? as u32;
                for &piece in &m.owners[hit.pattern().as_usize()] {
                    if acc.wants(piece) {
                        hits.push((w, piece));
                    }
                }
            }
            if hits.is_empty() {
                continue;
            }
            hits.sort_unstable();
            hits.dedup();
            let word = |w: u32| ranges.get(w as usize).copied().ok_or_else(corrupt);
            let (lo, hi) = (word(hits[0].0)?.0, word(hits[hits.len() - 1].0)?.1);
            let mut post = vec![0u8; (hi - lo) as usize];
            read_at(&self.file, &mut post, b.off + u64::from(b.vocab) + u64::from(b.lens) + u64::from(lo))?;
            let mut i = 0;
            while i < hits.len() {
                let w = hits[i].0;
                let (s, e) = word(w)?;
                let bytes = &post[(s - lo) as usize..(e - lo) as usize];
                let pieces: Vec<usize> =
                    hits[i..].iter().take_while(|h| h.0 == w).map(|h| h.1).filter(|&p| acc.charge(p, bytes.len())).collect();
                i += hits[i..].iter().take_while(|h| h.0 == w).count();
                if pieces.is_empty() {
                    continue;
                }
                let ok = decode_ids(bytes, self.min, self.end, |id| {
                    for &p in &pieces {
                        acc.set(p, id);
                    }
                });
                if !ok {
                    return Err(corrupt());
                }
            }
        }
        Ok(())
    }
}

/// Writes a segment: words in byte order, each with its ascending ids.
struct SegWriter {
    tmp: PathBuf,
    out: BufWriter<File>,
    pos: u64,
    min: u32,
    end: u32,
    vocab: Vec<u8>,
    lens: Vec<u8>,
    post: Vec<u8>,
    first: Vec<u8>,
    words: u32,
    blocks: u32,
    table: Vec<u8>,
    words_total: u64,
}

impl SegWriter {
    fn new(dir: &Path, serial: u64, min: u32, end: u32) -> io::Result<SegWriter> {
        let tmp = seg_path(dir, serial).with_extension("seg.tmp");
        let out = BufWriter::with_capacity(1 << 16, File::create(&tmp)?);
        Ok(SegWriter {
            tmp,
            out,
            pos: 0,
            min,
            end,
            vocab: Vec::new(),
            lens: Vec::new(),
            post: Vec::new(),
            first: Vec::new(),
            words: 0,
            blocks: 0,
            table: Vec::new(),
            words_total: 0,
        })
    }

    fn add(&mut self, word: &[u8], ids: impl IntoIterator<Item = u32>) -> io::Result<()> {
        let before = self.post.len();
        encode_ids(&mut self.post, ids, self.min);
        if self.post.len() == before {
            return Ok(()); // no live document holds it
        }
        if self.vocab.is_empty() {
            self.vocab.push(b'\n');
            self.first = word.to_vec();
        }
        self.vocab.extend_from_slice(word);
        self.vocab.push(b'\n');
        put_varint(&mut self.lens, (self.post.len() - before) as u64);
        self.words += 1;
        self.words_total += 1;
        if self.vocab.len() >= BLOCK_VOCAB || self.post.len() >= BLOCK_POSTINGS {
            self.flush_block()?;
        }
        Ok(())
    }

    fn flush_block(&mut self) -> io::Result<()> {
        if self.words == 0 {
            return Ok(());
        }
        let sum = {
            let mut head = Vec::with_capacity(self.vocab.len() + self.lens.len());
            head.extend_from_slice(&self.vocab);
            head.extend_from_slice(&self.lens);
            fnv32(&head)
        };
        self.table.extend_from_slice(&(self.first.len() as u16).to_le_bytes());
        self.table.extend_from_slice(&self.first);
        self.table.extend_from_slice(&self.pos.to_le_bytes());
        self.table.extend_from_slice(&(self.vocab.len() as u32).to_le_bytes());
        self.table.extend_from_slice(&(self.lens.len() as u32).to_le_bytes());
        self.table.extend_from_slice(&(self.post.len() as u32).to_le_bytes());
        self.table.extend_from_slice(&self.words.to_le_bytes());
        self.table.extend_from_slice(&sum.to_le_bytes());
        for part in [&self.vocab, &self.lens, &self.post] {
            self.out.write_all(part)?;
            self.pos += part.len() as u64;
        }
        self.vocab.clear();
        self.lens.clear();
        self.post.clear();
        self.words = 0;
        self.blocks += 1;
        Ok(())
    }

    /// Ends the segment with the documents that have an over-long word
    /// (`long`, ascending) and `docs` documents in all, syncs it and puts it
    /// in place. `None` (and no file) when it holds nothing.
    fn finish(mut self, dir: &Path, serial: u64, long: &[u32], docs: u32) -> io::Result<Option<Segment>> {
        self.flush_block()?;
        if self.words_total == 0 && long.is_empty() {
            drop(self.out);
            let _ = fs::remove_file(&self.tmp);
            return Ok(None);
        }
        let mut long_bytes = Vec::new();
        encode_ids(&mut long_bytes, long.iter().copied(), self.min);
        let long_off = self.pos;
        self.out.write_all(&long_bytes)?;
        self.pos += long_bytes.len() as u64;
        let table_off = self.pos;
        self.out.write_all(&self.table)?;
        let mut t = Vec::with_capacity(TRAILER);
        t.extend_from_slice(&SEG_MAGIC);
        for v in [VERSION, self.min, self.end, docs, self.blocks] {
            t.extend_from_slice(&v.to_le_bytes());
        }
        t.extend_from_slice(&table_off.to_le_bytes());
        t.extend_from_slice(&(self.table.len() as u32).to_le_bytes());
        t.extend_from_slice(&long_off.to_le_bytes());
        t.extend_from_slice(&(long_bytes.len() as u32).to_le_bytes());
        t.extend_from_slice(&fnv(&self.table).to_le_bytes());
        let sum = fnv(&t);
        t.extend_from_slice(&sum.to_le_bytes());
        self.out.write_all(&t)?;
        let file = self.out.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&self.tmp, seg_path(dir, serial))?;
        Segment::open(dir, serial).map(Some)
    }
}

/// Walks a segment's words in order (for a merge).
struct Cursor<'a> {
    seg: &'a Segment,
    block: usize,
    buf: Vec<u8>,
    words: Vec<(usize, usize)>,
    posts: Vec<(u32, u32)>,
    k: usize,
}

impl<'a> Cursor<'a> {
    fn new(seg: &'a Segment) -> io::Result<Cursor<'a>> {
        let mut c = Cursor { seg, block: 0, buf: Vec::new(), words: Vec::new(), posts: Vec::new(), k: 0 };
        c.load()?;
        Ok(c)
    }

    fn load(&mut self) -> io::Result<()> {
        self.k = 0;
        self.words.clear();
        self.posts.clear();
        while self.words.is_empty() && self.block < self.seg.blocks.len() {
            let b = &self.seg.blocks[self.block];
            self.block += 1;
            let (buf, ranges) = self.seg.read_block(b, true)?;
            let vocab = &buf[..b.vocab as usize];
            let mut start = 1;
            for nl in memchr::memchr_iter(b'\n', &vocab[1..]) {
                self.words.push((start, nl + 1));
                start = nl + 2;
            }
            if self.words.len() != ranges.len() {
                return Err(corrupt());
            }
            self.posts = ranges;
            self.buf = buf;
        }
        Ok(())
    }

    fn word(&self) -> Option<&[u8]> {
        self.words.get(self.k).map(|&(s, e)| &self.buf[s..e])
    }

    fn postings(&self) -> &[u8] {
        let b = &self.seg.blocks[self.block - 1];
        let base = b.vocab as usize + b.lens as usize;
        let (s, e) = self.posts[self.k];
        &self.buf[base + s as usize..base + e as usize]
    }

    fn advance(&mut self) -> io::Result<()> {
        self.k += 1;
        if self.k >= self.words.len() {
            self.load()?;
        }
        Ok(())
    }
}

/// Merges `segs` (adjacent, ascending ids) into segment `serial`, keeping only
/// the ids set in `live`.
fn merge_segments(dir: &Path, serial: u64, segs: &[Arc<Segment>], live: &[u64]) -> io::Result<Option<Segment>> {
    let (min, end) = (segs[0].min, segs[segs.len() - 1].end);
    let mut w = SegWriter::new(dir, serial, min, end)?;
    let mut cursors = segs.iter().map(|s| Cursor::new(s)).collect::<io::Result<Vec<_>>>()?;
    let mut ids: Vec<u32> = Vec::new();
    while let Some(word) = cursors.iter().filter_map(Cursor::word).min().map(<[u8]>::to_vec) {
        ids.clear();
        for c in &mut cursors {
            if c.word() == Some(word.as_slice()) {
                let ok = decode_ids(c.postings(), c.seg.min, c.seg.end, |id| {
                    if has_bit(live, id) {
                        ids.push(id);
                    }
                });
                if !ok {
                    return Err(corrupt());
                }
                c.advance()?;
            }
        }
        w.add(&word, ids.iter().copied())?;
    }
    let mut long = Vec::new();
    for s in segs {
        if !decode_ids(&s.read_long()?, s.min, s.end, |id| {
            if has_bit(live, id) {
                long.push(id);
            }
        }) {
            return Err(corrupt());
        }
    }
    let docs = count_bits(live, min, end);
    w.finish(dir, serial, &long, docs)
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct SegRef {
    serial: u64,
    min: u32,
    end: u32,
    bytes: u64,
}

/// A document of the manifest.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct DocRec {
    key: u64,
    id: u32,
    size: u64,
    mtime: i64,
    fp: u64,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    next_serial: u64,
    next_id: u32,
    segments: Vec<SegRef>,
    docs: Vec<DocRec>,
}

/// What the content store holds for one document: its path's [`key`], size,
/// modification time, and a checksum of the stored text (0 for "no text").
pub type Stamp = (u64, u64, i64, u64);

struct PendingDoc {
    id: u32,
    list: Box<str>,
    long: bool,
}

#[derive(Default)]
struct State {
    /// Documents indexed for their current stored text: key → id.
    ids: Ids,
    next_id: u32,
    /// Ids from here on are in `pending`, the ones before in `segments`.
    pending_from: u32,
    pending: Vec<PendingDoc>,
    pending_bytes: usize,
    /// Documents being written as a segment (still answered from here).
    writing: Option<Arc<Vec<PendingDoc>>>,
    segments: Vec<Arc<Segment>>,
    next_serial: u64,
    /// A damaged segment was found: answer nothing until [`Index::reset`].
    broken: bool,
}

/// How big the word index is, for logs and the bench.
#[derive(Debug, Clone, Copy, Default)]
pub struct WordStats {
    /// Documents indexed for their current text.
    pub docs: usize,
    pub segments: usize,
    /// Size of the segment files.
    pub bytes: u64,
    pub pending: usize,
    /// About how much memory it holds.
    pub heap: usize,
    /// A damaged segment was found (the next warm-up rebuilds it).
    #[cfg_attr(not(windows), allow(dead_code))] // the Windows UI's log
    pub broken: bool,
}

pub struct Index {
    dir: PathBuf,
    state: Mutex<State>,
    /// One merge at a time.
    merging: Mutex<()>,
    /// One manifest write at a time (taken before the store's lock).
    manifest: Mutex<()>,
}

/// A merge chosen by [`Index::plan_merge`].
pub struct MergeJob {
    from: usize,
    segs: Vec<Arc<Segment>>,
    live: Vec<u64>,
    serial: u64,
}

impl Index {
    /// Opens the index in `dir` for a content store holding `store` (one
    /// [`Stamp`] per document). Only documents whose stamp the manifest
    /// agrees with count as indexed. Never fails: a missing or damaged index
    /// is an empty one.
    pub fn open(dir: &Path, store: impl Iterator<Item = Stamp>) -> Index {
        let mut st = State::default();
        let mut docs = Vec::new();
        match Self::load(dir) {
            Some((m, segs)) => {
                st.next_id = segs.last().map_or(0, |s| s.end).max(m.next_id);
                st.next_serial = m.next_serial.max(segs.iter().map(|s| s.serial + 1).max().unwrap_or(0));
                st.segments = segs;
                docs = m.docs;
            }
            None => st.next_serial = 1,
        }
        docs.sort_unstable_by_key(|d| d.key);
        let in_segments = st.segments.last().map_or(0, |s| s.end);
        for (key, size, mtime, fp) in store {
            if let Ok(i) = docs.binary_search_by_key(&key, |d| d.key) {
                let d = docs[i];
                if (d.size, d.mtime, d.fp) == (size, mtime, fp) && d.id < in_segments {
                    st.ids.insert(key, d.id);
                }
            }
        }
        drop(docs);
        if st.ids.is_empty() {
            st.segments.clear();
        }
        st.pending_from = st.next_id;
        let index = Index { dir: dir.to_path_buf(), state: Mutex::new(st), merging: Mutex::new(()), manifest: Mutex::new(()) };
        index.remove_strays();
        index
    }

    fn load(dir: &Path) -> Option<(Manifest, Vec<Arc<Segment>>)> {
        let bytes = fs::read(dir.join(MANIFEST)).ok()?;
        let (payload, sum) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
        if fnv(payload).to_le_bytes() != sum {
            return None;
        }
        let m: Manifest = bincode::deserialize(payload).ok()?;
        if m.version != VERSION {
            return None;
        }
        let mut segs: Vec<Arc<Segment>> = Vec::new();
        for r in &m.segments {
            let s = Segment::open(dir, r.serial).ok()?;
            let after = segs.last().map_or(0, |p| p.end);
            if (s.min, s.end, s.bytes) != (r.min, r.end, r.bytes) || s.min < after {
                return None;
            }
            segs.push(Arc::new(s));
        }
        Some((m, segs))
    }

    /// Deletes the segment files (and temp files) the index does not use.
    fn remove_strays(&self) {
        let keep: HashSet<PathBuf> = self.lock().segments.iter().map(|s| s.path.clone()).collect();
        let Ok(list) = fs::read_dir(&self.dir) else { return };
        for e in list.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ours = name.starts_with(SEG_PREFIX) || name == format!("{MANIFEST}.tmp");
            if ours && !keep.contains(&e.path()) {
                let _ = fs::remove_file(e.path());
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The document at `key` now has `words` (its old id, if any, becomes
    /// a tombstone).
    pub fn replace(&self, key: u64, words: Words) {
        let mut st = self.lock();
        if st.next_id == u32::MAX {
            st.ids.remove(&key);
            return;
        }
        let id = st.next_id;
        st.next_id += 1;
        st.ids.insert(key, id);
        st.pending_bytes += words.list.len() + std::mem::size_of::<PendingDoc>();
        st.pending.push(PendingDoc { id, list: words.list.into_boxed_str(), long: words.long });
    }

    /// The document at `key` is gone from the store.
    pub fn remove(&self, key: u64) {
        self.lock().ids.remove(&key);
    }

    pub fn is_indexed(&self, key: u64) -> bool {
        self.lock().ids.contains_key(&key)
    }

    /// Enough documents wait in memory to be written as a segment.
    pub fn pending_full(&self) -> bool {
        let st = self.lock();
        st.pending.len() >= PENDING_DOCS || st.pending_bytes >= PENDING_BYTES
    }

    /// Writes the documents waiting in memory as a segment, when there are
    /// enough of them (or any, with `force`), without holding the index
    /// meanwhile: they are still answered from memory until the segment is
    /// in place. True when the segments changed (the manifest must be
    /// written). On a write error the documents are no longer indexed
    /// (searches read them in full). One write at a time: false while
    /// another runs.
    pub fn write_pending(&self, force: bool) -> io::Result<bool> {
        let (docs, min, end, serial, live) = {
            let mut st = self.lock();
            let full = st.pending.len() >= PENDING_DOCS || st.pending_bytes >= PENDING_BYTES;
            if st.writing.is_some() || st.pending.is_empty() || !(force || full) {
                return Ok(false);
            }
            let docs = Arc::new(std::mem::take(&mut st.pending));
            st.pending_bytes = 0;
            st.writing = Some(docs.clone());
            let serial = st.next_serial;
            st.next_serial += 1;
            let live = live_bits(&st.ids, st.next_id);
            (docs, st.pending_from, st.next_id, serial, live)
        };
        let written = write_pending_segment(&self.dir, serial, min, end, &docs, &live);
        let mut st = self.lock();
        st.writing = None;
        // (After a reset meanwhile it is past `end` already.)
        st.pending_from = st.pending_from.max(end);
        match written {
            Ok(seg) => {
                st.segments.extend(seg.map(Arc::new));
                Ok(true)
            }
            Err(e) => {
                st.ids.retain(|_, id| !(min..end).contains(id));
                Err(e)
            }
        }
    }

    /// The manifest for the store as `store` lists it now (call with the
    /// store's lock held, so the two agree), sealed with its checksum.
    pub fn manifest_bytes(&self, store: impl Iterator<Item = Stamp>) -> io::Result<Vec<u8>> {
        let st = self.lock();
        let docs: Vec<DocRec> = store
            .filter_map(|(key, size, mtime, fp)| {
                let id = *st.ids.get(&key)?;
                (id < st.pending_from).then_some(DocRec { key, id, size, mtime, fp })
            })
            .collect();
        let segments = st.segments.iter().map(|s| SegRef { serial: s.serial, min: s.min, end: s.end, bytes: s.bytes }).collect();
        let m = Manifest { version: VERSION, next_serial: st.next_serial, next_id: st.next_id, segments, docs };
        drop(st);
        let mut bytes = bincode::serialize(&m).map_err(io::Error::other)?;
        bytes.extend_from_slice(&fnv(&bytes).to_le_bytes());
        Ok(bytes)
    }

    /// Held while a manifest is built and written (before the store's lock).
    pub fn manifest_lock(&self) -> MutexGuard<'_, ()> {
        self.manifest.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn save_manifest(&self, bytes: &[u8]) -> io::Result<()> {
        let tmp = self.dir.join(format!("{MANIFEST}.tmp"));
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, self.dir.join(MANIFEST))
    }

    #[cfg(test)]
    pub fn segments(&self) -> usize {
        self.lock().segments.len()
    }

    /// The merge to do now, if any. While the warm-up runs (`idle` false)
    /// only past [`MAX_SEGMENTS`]; when idle, whenever there are small
    /// segments to fold or a fifth of the documents are tombstones. The
    /// first (biggest) segment is rewritten only when the others together
    /// reach half its size, or for the tombstones.
    pub fn plan_merge(&self, idle: bool) -> Option<MergeJob> {
        let mut st = self.lock();
        let n = st.segments.len();
        if n == 0 || (!idle && n <= MAX_SEGMENTS) {
            return None;
        }
        let live = live_bits(&st.ids, st.next_id);
        let docs: u64 = st.segments.iter().map(|s| u64::from(s.docs)).sum();
        let alive: u64 = st.segments.iter().map(|s| u64::from(count_bits(&live, s.min, s.end))).sum();
        let dead = docs - alive.min(docs);
        let rest: u64 = st.segments[1..].iter().map(|s| s.bytes).sum();
        // A fifth of the documents are tombstones: rewrite everything.
        let tombstones = dead > 0 && dead * 5 >= docs;
        let from = usize::from(!tombstones && rest * 2 < st.segments[0].bytes);
        // One segment is rewritten on its own only for its tombstones.
        if n - from < 2 && !tombstones {
            return None;
        }
        let serial = st.next_serial;
        st.next_serial += 1;
        Some(MergeJob { from, segs: st.segments[from..].to_vec(), live, serial })
    }

    /// Runs `job` (without holding the index), then puts the merged segment
    /// in place. Returns the files to delete once the manifest no longer
    /// names them.
    pub fn run_merge(&self, job: MergeJob) -> io::Result<Vec<PathBuf>> {
        let merged = match merge_segments(&self.dir, job.serial, &job.segs, &job.live) {
            Ok(m) => m,
            Err(e) => {
                let _ = fs::remove_file(seg_path(&self.dir, job.serial).with_extension("seg.tmp"));
                if e.kind() == io::ErrorKind::InvalidData {
                    self.lock().broken = true;
                }
                return Err(e);
            }
        };
        let mut st = self.lock();
        let same = st.segments.get(job.from..job.from + job.segs.len()).is_some_and(|now| now.iter().zip(&job.segs).all(|(a, b)| a.serial == b.serial));
        if !same {
            drop(st);
            if let Some(m) = merged {
                let _ = fs::remove_file(&m.path);
            }
            return Ok(Vec::new());
        }
        let old: Vec<PathBuf> = job.segs.iter().map(|s| s.path.clone()).collect();
        st.segments.splice(job.from..job.from + job.segs.len(), merged.map(Arc::new));
        Ok(old)
    }

    /// Takes one merge slot, or `None` while another merge runs.
    pub fn merge_slot(&self) -> Option<MutexGuard<'_, ()>> {
        self.merging.try_lock().ok()
    }

    /// Forgets everything (after damage): every document is indexed again
    /// by the next warm-up. Returns the files to delete.
    pub fn reset(&self) -> Vec<PathBuf> {
        let mut st = self.lock();
        let old = st.segments.drain(..).map(|s| s.path.clone()).collect();
        st.ids = Ids::default();
        st.pending.clear();
        st.pending_bytes = 0;
        st.writing = None;
        st.pending_from = st.next_id;
        st.broken = false;
        old
    }

    pub fn is_broken(&self) -> bool {
        self.lock().broken
    }

    /// The ids of the documents indexed for their current text, read under
    /// the index's lock (call with the store's lock held, or none).
    pub fn ids(&self) -> IdsGuard<'_> {
        IdsGuard(self.lock())
    }

    /// The documents that may match `plan`.
    pub fn candidates(&self, plan: &Plan) -> Found {
        if plan.conds.is_empty() {
            return Found::Everything;
        }
        let Some(m) = Matcher::new(plan) else { return Found::Everything };
        let (segs, end, mut acc) = {
            let st = self.lock();
            if st.broken {
                return Found::Unavailable;
            }
            let mut acc = Acc::new(plan.pieces.len(), st.next_id);
            for d in st.writing.iter().flat_map(|w| w.iter()).chain(&st.pending) {
                if d.long {
                    acc.set_all(d.id);
                } else {
                    m.doc(d.list.as_bytes(), |p| acc.set(p, d.id));
                }
            }
            (st.segments.clone(), st.next_id, acc)
        };
        for s in &segs {
            if s.lookup(&m, &mut acc).is_err() {
                self.lock().broken = true;
                return Found::Unavailable;
            }
        }
        match acc.evaluate(plan, end) {
            Some(bits) => Found::Ids { bits, end },
            None => Found::Everything,
        }
    }

    pub fn stats(&self) -> WordStats {
        let st = self.lock();
        let blocks: usize = st.segments.iter().map(|s| s.blocks.capacity() * std::mem::size_of::<Block>()).sum();
        WordStats {
            docs: st.ids.len(),
            segments: st.segments.len(),
            bytes: st.segments.iter().map(|s| s.bytes).sum(),
            pending: st.pending.len(),
            // hashbrown: the (key, id) pair, 16 bytes aligned, and a control byte.
            heap: st.ids.capacity() * 17 + st.pending_bytes + blocks,
            broken: st.broken,
        }
    }

    /// Lets go of spare room (for the idle trim).
    pub fn shrink(&self) {
        let mut st = self.lock();
        st.ids.shrink_to_fit();
        st.pending.shrink_to_fit();
    }
}

pub struct IdsGuard<'a>(MutexGuard<'a, State>);

impl IdsGuard<'_> {
    pub fn get(&self, key: u64) -> Option<u32> {
        self.0.ids.get(&key).copied()
    }
}

/// Writes the documents in `pending` whose ids are in `live` as segment
/// `serial`.
fn write_pending_segment(dir: &Path, serial: u64, min: u32, end: u32, pending: &[PendingDoc], live: &[u64]) -> io::Result<Option<Segment>> {
    let docs: Vec<&PendingDoc> = pending.iter().filter(|d| has_bit(live, d.id)).collect();
    // Word → ids (ascending: the documents are in id order); then only the
    // distinct words are sorted.
    let mut by_word: HashMap<&[u8], Vec<u32>> = HashMap::new();
    let mut long = Vec::new();
    for d in &docs {
        if d.long {
            long.push(d.id);
        }
        for w in d.list.as_bytes().split(|&b| b == b'\n').filter(|w| !w.is_empty()) {
            by_word.entry(w).or_default().push(d.id);
        }
    }
    let mut words: Vec<(&[u8], Vec<u32>)> = by_word.into_iter().collect();
    words.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let mut w = SegWriter::new(dir, serial, min, end)?;
    for (word, ids) in &words {
        w.add(word, ids.iter().copied())?;
    }
    w.finish(dir, serial, &long, docs.len() as u32)
}

#[cfg(test)]
mod tests;
