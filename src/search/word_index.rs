//! The content index's word index as a filter: a search with it finds
//! exactly what a full scan of the stored texts finds (randomised corpora
//! and queries), and the bench (`bench_word_index`, ignored) measures it on
//! 300k synthetic documents.

use super::*;
use crate::content_cache::ContentCache;
use std::path::Path;
use std::time::Instant;

/// A result: (entry, count, snippet, lines).
type Hit = (u32, u32, String, Vec<(u32, String)>);

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Words the queries look for, with how many documents in 10,000 hold
/// them (one to three times): hamza seats in their spellings, articles,
/// words inside other words («باريس»), Latin with case and punctuation.
/// Typical words of a search: each is in 0.2 to 3% of the documents.
const PLANTED: &[(&str, usize)] = &[
    ("تقرير", 300),
    ("التقرير", 150),
    ("والتقارير", 50),
    ("مالي", 200),
    ("رئيس", 100),
    ("الرئيس", 80),
    ("الريس", 20),
    ("باريس", 30),
    ("مسئول", 40),
    ("مسؤول", 60),
    ("المسوول", 10),
    ("شيء", 150),
    ("شئ", 30),
    ("نهائي", 60),
    ("نهاية", 80),
    ("فاتورة", 50),
    ("Report", 200),
    ("reports", 80),
    ("annual", 100),
    ("e-mail", 40),
    ("email", 60),
    ("invoice", 50),
    ("2024", 100),
    ("٢٠٢٤", 30),
    ("عبد", 100),
    ("أحمد", 150),
    ("إدارة", 120),
    ("ﻻ", 20),
    ("مُدير", 80),
];

const ARABIC: &[char] = &[
    'ا', 'أ', 'إ', 'آ', 'ب', 'ت', 'ث', 'ج', 'ح', 'خ', 'د', 'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', 'ط', 'ظ', 'ع', 'غ', 'ف', 'ق', 'ك',
    'ل', 'م', 'ن', 'ه', 'و', 'ي', 'ة', 'ى', 'ئ', 'ؤ', 'ء',
];

/// A vocabulary of `n` synthetic words: Arabic (most), English, numbers.
fn vocabulary(rng: &mut Rng, n: usize) -> Vec<String> {
    (0..n)
        .map(|_| match rng.below(10) {
            0..=5 => (0..2 + rng.below(7)).map(|_| ARABIC[rng.below(ARABIC.len())]).collect(),
            6..=8 => (0..2 + rng.below(9)).map(|_| (b'a' + rng.below(26) as u8) as char).collect(),
            _ => rng.below(1_000_000).to_string(),
        })
        .collect()
}

/// A document of about `len` bytes: words drawn from `vocab` (roughly
/// Zipf: rank `n^u`), with articles, punctuation and lines, and the
/// [`PLANTED`] words in `boost` times their share of the documents.
fn document(rng: &mut Rng, vocab: &[String], len: usize, boost: usize) -> String {
    let mut words: Vec<String> = Vec::with_capacity(len / 6);
    let mut size = 0;
    while size < len {
        let rank = (vocab.len() as f64).powf(rng.unit()) as usize - 1;
        let w = &vocab[rank.min(vocab.len() - 1)];
        let mut word = String::new();
        if rng.below(8) == 0 && !w.is_ascii() {
            word.push_str(["ال", "و", "وال", "بال"][rng.below(4)]);
        }
        word.push_str(w);
        size += word.len() + 1;
        words.push(word);
    }
    for (w, per) in PLANTED {
        if rng.below(10_000) < per * boost {
            for _ in 0..1 + rng.below(3) {
                let at = rng.below(words.len() + 1);
                words.insert(at, w.to_string());
            }
        }
    }
    let mut s = String::with_capacity(size + 64);
    for w in &words {
        s.push_str(w);
        s.push_str(match rng.below(20) {
            0 => ".\n",
            1 => "، ",
            2 => " - ",
            3 => ": ",
            _ => " ",
        });
    }
    s
}

fn entry(i: usize, len: usize) -> Entry {
    Entry { path: format!("D:\\docs\\{}\\{i}.txt", i % 97).into(), size: len as u64 + 1, mtime: 1_700_000_000 + i as i64, is_dir: false, norm_name: Box::default() }
}

/// Fills `cache` with `n` documents from `docs(i)`; returns their entries.
fn fill(cache: &ContentCache, n: usize, docs: &(dyn Fn(usize) -> String + Sync)) -> Vec<Entry> {
    (0..n)
        .into_par_iter()
        .map(|i| {
            let text = docs(i);
            let e = entry(i, text.len());
            // Some documents have no text, like scanned PDFs without OCR.
            cache.put(&e.path, e.size, e.mtime, (i % 50 != 7).then_some(text.as_str()));
            e
        })
        .collect()
}

/// The hits of `q` over `entries`, read from `cache` as their entries say,
/// with the word index (`indexed`) or without: sorted, with the progress.
/// Every result keeps its lines (which ones do otherwise depends on the
/// order the threads find them).
fn run(entries: &[Entry], cache: &ContentCache, q: &Query, indexed: bool) -> (Vec<Hit>, ContentProgress) {
    run_with(entries, cache, q, indexed, HitLimits { lined: usize::MAX, hits: MAX_CONTENT_HITS })
}

fn run_with(
    entries: &[Entry],
    cache: &ContentCache,
    q: &Query,
    indexed: bool,
    limits: HitLimits,
) -> (Vec<Hit>, ContentProgress) {
    let progress = ContentProgress::default();
    let hits = Mutex::new(Vec::new());
    let text_of = |e: &Entry| match cache.get(&e.path, e.size, e.mtime) {
        Some(t) => (t, true),
        None => (None, false),
    };
    let index = WordFilter { cache, recheck: None };
    search_documents(entries, q, "", &progress, &hits, limits, &text_of, indexed.then_some(index));
    let mut out: Vec<_> = hits.into_inner().unwrap().into_iter().map(|h| (h.index, h.count, h.snippet, h.lines)).collect();
    out.sort_by_key(|h| h.0);
    (out, progress)
}

fn at(s: &str) -> Query {
    Query::parse_at(s, 1_700_000_000, 0)
}

/// The queries of the test and the bench.
const QUERIES: &[&str] = &[
    "تقرير",
    "report",
    "رئيس",
    "ريس",
    "مسوول",
    "مسؤول",
    "شيء",
    "نهائي",
    "\"annual report\"",
    "ww:report",
    "report -annual",
    "تقرير مالي",
    "\"تقرير مالي\"~3",
    "ww:ريس",
    "التقرير",
    "e-mail",
    "2024",
    "invoice|فاتورة",
    "مدير ادارة",
    "port",
    "franco:3abd",
    "لا",
    "ﻻ",
    "REPORTS",
    "\"report, reports\"",
    "ww:\"annual report\"",
    "تقارير -مالي",
    "regex:rep[o]rt",
];

fn temp_dir(name: &str) -> crate::test_temp::TempPath {
    let dir = std::env::temp_dir().join(format!("bahth_wordidx_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::test_temp::TempPath::at(dir)
}

/// Queries made of the corpus's own words and pieces of them, with every
/// operator.
fn random_queries(rng: &mut Rng, texts: &[String], n: usize) -> Vec<String> {
    let mut out = Vec::new();
    while out.len() < n {
        let text = &texts[rng.below(texts.len())];
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let mut word = || {
            let w = words[rng.below(words.len())];
            let chars: Vec<char> = w.chars().collect();
            match rng.below(4) {
                // A piece of a word.
                0 if chars.len() > 2 => {
                    let a = rng.below(chars.len() - 1);
                    chars[a..a + 1 + rng.below(chars.len() - a)].iter().collect()
                }
                _ => w.to_string(),
            }
        };
        let (a, b) = (word(), word());
        let pick = rng.below(9);
        out.push(match pick {
            0 => a,
            1 => format!("{a} {b}"),
            2 => format!("\"{a} {b}\""),
            3 => format!("ww:{a}"),
            4 => format!("{a} -{b}"),
            5 => format!("\"{a} {b}\"~{}", 1 + rng.below(5)),
            6 => format!("{a}|{b}"),
            7 => format!("ww:\"{a} {b}\""),
            _ => format!("!{b} {a}"),
        });
    }
    out
}

#[test]
fn the_word_index_finds_what_a_full_scan_finds() {
    let dir = temp_dir("same");
    let mut rng = Rng(0x5eed);
    let vocab = vocabulary(&mut rng, 3000);
    let lens: Vec<usize> = (0..800).map(|_| 50 + rng.below(600)).collect();
    let texts: Vec<String> = lens.iter().map(|&l| document(&mut rng, &vocab, l, 10)).collect();
    let mut texts = texts;
    // An archive (members matched one by one), a document of one long
    // word, a stale one.
    texts[3] = "\u{1F}\u{FDD0}تقارير\\a.txt\nتقرير مالي نهائي\n\u{1F}\u{FDD0}b.txt\nannual report\n".into();
    texts[4] = format!("{} report", "x".repeat(300));
    let cache = ContentCache::open(&dir).unwrap();
    let mut entries = fill(&cache, texts.len(), &|i| texts[i].clone());
    // Indexed in memory, then partly written as segments, then reopened.
    let mut queries: Vec<String> = QUERIES.iter().map(|s| s.to_string()).collect();
    queries.extend(random_queries(&mut rng, &texts, 120));
    let check = |cache: &ContentCache, entries: &[Entry], stage: &str| {
        let mut narrowed = 0;
        for text in &queries {
            let q = at(text);
            let (full, _) = run(entries, cache, &q, false);
            let (fast, p) = run(entries, cache, &q, true);
            if fast != full {
                let ids = |h: &[Hit]| h.iter().map(|h| h.0).collect::<Vec<_>>();
                let (f, s) = (ids(&full), ids(&fast));
                let missed: Vec<_> = f.iter().filter(|i| !s.contains(i)).collect();
                let extra: Vec<_> = s.iter().filter(|i| !f.contains(i)).collect();
                panic!("{stage}: {text}: missed {missed:?}, extra {extra:?}, plan {:?}", word_plan(&q.for_documents()));
            }
            narrowed += usize::from(p.narrowed.load(Ordering::Relaxed));
        }
        assert!(narrowed > queries.len() / 2, "{stage}: the index narrowed only {narrowed} queries");
    };
    check(&cache, &entries, "in memory");
    cache.flush().unwrap();
    cache.maintain_words().unwrap();
    assert!(cache.word_stats().segments >= 1);
    // Changed and deleted documents: tombstones and new pending ones.
    for i in (0..texts.len()).step_by(37) {
        cache.put(&entries[i].path, entries[i].size, entries[i].mtime, Some("رئيس التقرير النهائي report"));
    }
    // A file changed on disk since it was stored: read as before (here,
    // the stamp in the entry no longer matches: no text).
    entries[10].mtime += 5;
    check(&cache, &entries, "segments and pending");
    cache.flush().unwrap();
    cache.maintain_words().unwrap();
    drop(cache);
    let cache = ContentCache::open(&dir).unwrap();
    assert_eq!(cache.word_stats().docs, texts.len());
    check(&cache, &entries, "reopened");
    // The word index lost: every search reads everything, still the same.
    drop(cache);
    std::fs::remove_file(dir.join("words.man")).unwrap();
    let cache = ContentCache::open(&dir).unwrap();
    assert_eq!(cache.word_stats().docs, 0);
    let (_, p) = run(&entries, &cache, &at("تقرير"), true);
    assert_eq!(p.unindexed.load(Ordering::Relaxed), p.total.load(Ordering::Relaxed));
    // Built again from the stored texts (the warm-up does this).
    for e in &entries {
        cache.index_stored(&e.path);
    }
    cache.maintain_words().unwrap();
    check(&cache, &entries, "rebuilt");
}

#[test]
fn hamza_and_article_forms_are_looked_up() {
    let dir = temp_dir("forms");
    let cache = ContentCache::open(&dir).unwrap();
    // (Number 7 is stored as "no text", see `fill`.)
    let texts = ["قال الرئيس", "زار باريس", "والريس حضر", "المسئول هنا", "لا شيء", "مسؤول", "نهاية الأمر", "شئ ما", "شئ ما", "e-mail me"];
    let entries = fill(&cache, texts.len(), &|i| texts[i].to_string());
    for (q, want) in [
        // As typed it is found anywhere (also in «باريس»); its other
        // spellings only as words.
        ("ريس", vec![0, 1, 2]),
        ("رئيس", vec![0, 2]),
        ("مسوول", vec![3, 5]),
        ("شيء", vec![4, 8]),
        ("باريس", vec![1]),
        ("e-mail", vec![9]),
        ("ww:mail", vec![9]),
    ] {
        let (full, _) = run(&entries, &cache, &at(q), false);
        let (fast, p) = run(&entries, &cache, &at(q), true);
        assert_eq!(fast, full, "{q}");
        assert_eq!(fast.iter().map(|h| h.0 as usize).collect::<Vec<_>>(), want, "{q}");
        assert!(p.narrowed.load(Ordering::Relaxed), "{q}");
        assert_eq!(p.checked.load(Ordering::Relaxed), p.total.load(Ordering::Relaxed), "{q}: progress ends at the total");
    }
}

/// Run with `cargo test --release bench_word_index -- --ignored --nocapture`.
/// `BAHTH_BENCH_DOCS` (default 300000) documents of about 3.5 KB; the store
/// goes in the temp folder (or `BAHTH_BENCH_DIR`).
#[test]
#[ignore]
fn bench_word_index() {
    let n: usize = std::env::var("BAHTH_BENCH_DOCS").ok().and_then(|v| v.parse().ok()).unwrap_or(300_000);
    let base = std::env::var("BAHTH_BENCH_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    let dir = crate::test_temp::TempPath::at(base.join(format!("bahth_wordidx_bench_{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&*dir);
    let vocab = vocabulary(&mut Rng(7), 400_000);
    let doc = |i: usize| {
        let mut rng = Rng(0x1234_5678 ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let len = 700 + rng.below(5600);
        document(&mut rng, &vocab, len, 1)
    };
    let cache = ContentCache::open_with_limit(&dir, u64::MAX).unwrap();
    let t = Instant::now();
    let entries = fill(&cache, n, &doc);
    cache.flush().unwrap();
    cache.maintain_words().unwrap();
    let fill_ms = t.elapsed().as_millis();
    let raw: u64 = entries.iter().map(|e| e.size - 1).sum();
    let stats = cache.stats();
    let w = cache.word_stats();
    println!("documents: {n}, text {} MB (average {} bytes)", raw >> 20, raw / n as u64);
    println!("store filled with the word index in {fill_ms} ms (4 threads, extraction excluded)");
    println!("content.dat {} MB; word index {} MB in {} segments = {:.1}% of content.dat", stats.bytes >> 20, w.bytes >> 20, w.segments, 100.0 * w.bytes as f64 / stats.bytes as f64);

    // The upgrade: a store without the index, built from the stored texts.
    drop(cache);
    std::fs::remove_file(dir.join("words.man")).unwrap();
    let cache = ContentCache::open_with_limit(&dir, u64::MAX).unwrap();
    let t = Instant::now();
    entries.par_iter().for_each(|e| {
        cache.index_stored(&e.path);
    });
    cache.flush().unwrap();
    cache.maintain_words().unwrap();
    let w = cache.word_stats();
    println!("built from the stored texts in {} ms; {} MB in {} segments", t.elapsed().as_millis(), w.bytes >> 20, w.segments);
    // Memory: the store opened with and without its word index.
    drop(cache);
    let (_, with, cache) = crate::test_alloc::retained_of(|| ContentCache::open_with_limit(&dir, u64::MAX).unwrap());
    let est = cache.word_stats().heap;
    drop(cache);
    // Without it: its files moved aside (an open without them would delete them).
    let aside = dir.join("aside");
    std::fs::create_dir_all(&aside).unwrap();
    let ours: Vec<String> = std::fs::read_dir(&*dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.starts_with("words"))
        .collect();
    for f in &ours {
        std::fs::rename(dir.join(f), aside.join(f)).unwrap();
    }
    let (_, without, cache) = crate::test_alloc::retained_of(|| ContentCache::open_with_limit(&dir, u64::MAX).unwrap());
    drop(cache);
    for f in &ours {
        std::fs::rename(aside.join(f), dir.join(f)).unwrap();
    }
    let cache = ContentCache::open_with_limit(&dir, u64::MAX).unwrap();
    assert_eq!(cache.word_stats().docs, n);
    println!(
        "memory after open: store with word index {} MB, without {} MB: word index {} MB ({} bytes a document; estimate {} MB)",
        with >> 20,
        without >> 20,
        (with - without) >> 20,
        (with - without) / n,
        est >> 20
    );

    println!("{:<28} {:>9} {:>9} {:>9} {:>8} {:>9} {:>9}  same", "query", "full ms", "index ms", "filter ms", "read", "hits", "speed-up");
    for text in QUERIES {
        let q = at(text);
        let t = Instant::now();
        let (full, pf) = run_with(&entries, &cache, &q, false, HitLimits::DEFAULT);
        let full_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let (fast, p) = run_with(&entries, &cache, &q, true, HitLimits::DEFAULT);
        let fast_ms = t.elapsed().as_secs_f64() * 1000.0;
        // The filter alone: the word index's lookup and the documents kept.
        let all: Vec<u32> = (0..entries.len() as u32).collect();
        let t = Instant::now();
        let _ = cache.narrow(&entries, &all, &word_plan(&q.for_documents()));
        let filter_ms = t.elapsed().as_secs_f64() * 1000.0;
        // Which results keep their lines depends on the order they are
        // found, and so does which ones a capped search lists.
        let key = |h: &[Hit]| h.iter().map(|h| (h.0, h.1, h.2.clone())).collect::<Vec<_>>();
        let capped = pf.capped.load(Ordering::Relaxed) || p.capped.load(Ordering::Relaxed);
        let same = if capped { full.len() == fast.len() && pf.capped.load(Ordering::Relaxed) == p.capped.load(Ordering::Relaxed) } else { key(&fast) == key(&full) };
        let read = p.total.load(Ordering::Relaxed) - p.ruled_out.load(Ordering::Relaxed);
        let hits = format!("{}{}", full.len(), if capped { "+" } else { "" });
        println!("{text:<28} {full_ms:>9.0} {fast_ms:>9.0} {filter_ms:>9.0} {read:>8} {hits:>9} {:>8.1}x  {same}", full_ms / fast_ms);
        assert!(same, "{text}");
    }
    drop(cache);
    let _ = Path::new(&*dir);
}
