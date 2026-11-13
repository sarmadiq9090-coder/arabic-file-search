use super::*;
use crate::normalize::normalize;
use crate::search::split_words;

fn temp_dir(name: &str) -> crate::test_temp::TempPath {
    let dir = std::env::temp_dir().join(format!("bahth_words_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    crate::test_temp::TempPath::at(dir)
}

/// What `doc_words` must give: the distinct words of the normalised text.
fn expected_words(text: &str) -> (Vec<String>, bool) {
    let norm = normalize(text);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut long = false;
    for (_, w) in split_words(&norm) {
        if w.len() > MAX_WORD {
            long = true;
        } else if seen.insert(w.to_string()) {
            out.push(w.to_string());
        }
    }
    out.sort_unstable();
    (out, long)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Text mixing what normalisation folds, drops or splits on.
fn random_text(rng: &mut Rng, len: usize) -> String {
    const PIECES: &[&str] = &[
        "ا", "أ", "إ", "آ", "ة", "ه", "ى", "ي", "ئ", "ؤ", "و", "ء", "ل", "ك", "ک", "ی", "ﻻ", "ﷲ", "ﺑ", "ﹶ", "\u{064B}", "\u{0640}",
        "\u{0654}", "\u{200F}", "\u{1}", "\u{2}", "٣", "۴", "A", "b", "Z", "İ", "ß", "Σ", "é", "e\u{301}", "1", "_", "-", ".", " ",
        "\n", "\t", "،", "؟", "\u{1F}\u{FDD0}", "x", "ё", "Ё", "\u{0308}", "\u{0300}", "\u{0327}", "ç", "ş", "ğ", "\u{0711}",
        "\u{0730}", "ܐ", "Ａ", "ｶ", "カ", "か", "\u{3099}", "東", "京", "ไทย", "\u{0E34}", "क", "\u{093C}", "ड़", "ک", "گ", "ۆ", "ێ", "ڕ",
        "پ", "ٹ", "ے", "\u{06C1}", "ﬁ", "²", "Ⅻ", "ǅ",
    ];
    (0..len).map(|_| PIECES[rng.below(PIECES.len())]).collect()
}

#[test]
fn doc_words_are_the_words_of_the_normalised_text() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for round in 0..3000 {
        let len = rng.below(60);
        let text = random_text(&mut rng, len);
        let w = doc_words(&text);
        let (want, long) = expected_words(&text);
        assert_eq!(w.words(), want, "round {round}: {text:?}");
        assert_eq!(w.long, long, "{text:?}");
    }
    let long_word = "ب".repeat(MAX_WORD);
    let w = doc_words(&format!("قصير {long_word} آخر قصير"));
    assert!(w.long);
    assert_eq!(w.words(), ["اخر", "قصير"]);
    // A big document: more words than one chunk.
    let big: String = (0..3 * WORD_CHUNK).map(|i| format!("w{} ", i % (WORD_CHUNK + 7))).collect();
    let (want, _) = expected_words(&big);
    assert_eq!(doc_words(&big).words(), want);
    assert_eq!(doc_words(" .,- "), Words::none());
    assert_eq!(doc_words("Report REPORT report").words(), ["report"]);
}

#[test]
fn ids_round_trip_and_damage_is_found() {
    let ids = [5u32, 6, 9, 200, 70_000, 1 << 30];
    let mut b = Vec::new();
    encode_ids(&mut b, ids, 5);
    let mut got = Vec::new();
    assert!(decode_ids(&b, 5, (1 << 30) + 1, |i| got.push(i)));
    assert_eq!(got, ids);
    assert!(!decode_ids(&b, 5, 1 << 30, |_| {}), "an id past the end");
    assert!(!decode_ids(&[0, 0, 0], 0, 10, |_| {}), "zeros (a hole in the file)");
    assert!(!decode_ids(&[0x80], 0, 10, |_| {}), "cut short");
}

/// A plan of one condition: any of `alts`, each all of its pieces' patterns.
fn plan(alts: &[&[&[&str]]]) -> Plan {
    let mut p = Plan::default();
    let cond = alts
        .iter()
        .map(|alt| alt.iter().map(|pats| p.piece(pats.iter().map(|s| s.as_bytes().to_vec()).collect(), false)).collect())
        .collect();
    p.conds.push(cond);
    p
}

fn found(index: &Index, plan: &Plan) -> Option<Vec<u32>> {
    match index.candidates(plan) {
        Found::Ids { bits, end } => Some((0..end).filter(|&i| has_bit(&bits, i)).collect()),
        Found::Everything => None,
        Found::Unavailable => panic!("unavailable"),
    }
}

/// The documents (by key) a plan finds.
fn found_keys(index: &Index, plan: &Plan, keys: &[u64]) -> Vec<usize> {
    let bits = found(index, plan).unwrap();
    let ids = index.ids();
    keys.iter().enumerate().filter(|(_, k)| ids.get(**k).is_some_and(|id| bits.contains(&id))).map(|(i, _)| i).collect()
}

const DOCS: [&str; 5] = [
    "تقرير المبيعات السنوي",
    "the annual report, reporting period",
    "رئيس مجلس الإدارة والريس",
    "",
    "unrelated text",
];

fn fill(index: &Index) -> Vec<u64> {
    DOCS.iter()
        .enumerate()
        .map(|(i, t)| {
            let k = key(&format!("C:\\doc{i}.txt"));
            index.replace(k, doc_words(t));
            k
        })
        .collect()
}

fn stamps(keys: &[u64]) -> Vec<Stamp> {
    keys.iter().enumerate().map(|(i, &k)| (k, i as u64, 1_700_000_000 + i as i64, 77 + i as u64)).collect()
}

fn save(dir: &Path, index: &Index, keys: &[u64]) {
    let bytes = index.manifest_bytes(stamps(keys).into_iter()).unwrap();
    index.save_manifest(&bytes).unwrap();
    let _ = dir;
}

fn check_lookups(index: &Index, keys: &[u64]) {
    // Inside a word, at its start, at its end, the whole word.
    assert_eq!(found_keys(index, &plan(&[&[&["port"]]]), keys), [1]);
    assert_eq!(found_keys(index, &plan(&[&[&["\nport"]]]), keys), Vec::<usize>::new());
    assert_eq!(found_keys(index, &plan(&[&[&["\nreport\n"]]]), keys), [1]);
    assert_eq!(found_keys(index, &plan(&[&[&["ing\n"]]]), keys), [1]);
    assert_eq!(found_keys(index, &plan(&[&[&["مبيعات\n"]]]), keys), [0]);
    // Alternatives, pieces, prefixed whole words.
    assert_eq!(found_keys(index, &plan(&[&[&["تقرير"]], &[&["annual"], &["\nperiod\n"]]]), keys), [0, 1]);
    assert_eq!(found_keys(index, &plan(&[&[&["annual"], &["\nmissing"]]]), keys), Vec::<usize>::new());
    assert_eq!(found_keys(index, &plan(&[&[&["\nريس\n", "\nوالريس\n"]]]), keys), [2]);
    // Nothing matches.
    assert_eq!(found(index, &plan(&[&[&["zzz"]]])).unwrap(), Vec::<u32>::new());
}

#[test]
fn lookups_in_memory_on_disk_and_after_reopening() {
    let dir = temp_dir("lookups");
    let index = Index::open(&dir, std::iter::empty());
    let keys = fill(&index);
    check_lookups(&index, &keys);
    // Written as a segment: the same answers.
    assert!(index.write_pending(true).unwrap());
    assert_eq!(index.segments(), 1);
    assert_eq!(index.stats().pending, 0);
    check_lookups(&index, &keys);
    save(&dir, &index, &keys);
    drop(index);
    // Reopened for the same store.
    let index = Index::open(&dir, stamps(&keys).into_iter());
    assert_eq!(index.stats().docs, 5);
    check_lookups(&index, &keys);
    // A document whose stored text changed since is not indexed any more.
    let mut changed = stamps(&keys);
    changed[1].3 += 1;
    drop(index);
    let index = Index::open(&dir, changed.into_iter());
    assert!(!index.is_indexed(keys[1]));
    assert!(index.is_indexed(keys[0]));
    assert_eq!(found_keys(&index, &plan(&[&[&["port"]]]), &keys), Vec::<usize>::new());
}

#[test]
fn a_plan_that_narrows_nothing_says_so() {
    let dir = temp_dir("everything");
    let index = Index::open(&dir, std::iter::empty());
    fill(&index);
    assert!(matches!(index.candidates(&Plan::default()), Found::Everything));
    // One alternative of the condition may be anywhere: so may the term.
    let mut p = plan(&[&[&["port"]]]);
    p.conds[0].push(Vec::new());
    assert!(matches!(index.candidates(&p), Found::Everything));
}

#[test]
fn long_words_make_a_document_match_every_piece() {
    let dir = temp_dir("long");
    let index = Index::open(&dir, std::iter::empty());
    let k = key("C:\\blob.txt");
    index.replace(k, doc_words(&format!("short {}", "q".repeat(MAX_WORD + 1))));
    let p = plan(&[&[&["qqqq"]]]);
    assert_eq!(found(&index, &p).unwrap(), [0]);
    index.write_pending(true).unwrap();
    assert_eq!(found(&index, &p).unwrap(), [0]);
}

#[test]
fn replaced_and_removed_documents_are_tombstones_until_a_merge() {
    let dir = temp_dir("tombstones");
    let index = Index::open(&dir, std::iter::empty());
    let mut keys = Vec::new();
    // Many small segments.
    for seg in 0..5 {
        for i in 0..40 {
            let k = key(&format!("C:\\{seg}\\{i}.txt"));
            index.replace(k, doc_words(&format!("common word{i} seg{seg}")));
            keys.push(k);
        }
        index.write_pending(true).unwrap();
    }
    assert_eq!(index.segments(), 5);
    // A changed text, a deleted document.
    index.replace(keys[0], doc_words("changed"));
    index.remove(keys[1]);
    index.write_pending(true).unwrap();
    let p = plan(&[&[&["common"]]]);
    let before = found_keys(&index, &p, &keys);
    assert_eq!(before.len(), 198);
    assert!(!before.contains(&0) && !before.contains(&1));
    // Idle: the small segments fold into one; tombstones are dropped.
    let job = index.plan_merge(true).expect("a merge");
    let old = index.run_merge(job).unwrap();
    assert!(!old.is_empty());
    for f in old {
        fs::remove_file(f).unwrap();
    }
    assert!(index.segments() < 6);
    assert_eq!(found_keys(&index, &p, &keys), before);
    assert_eq!(found_keys(&index, &plan(&[&[&["\nchanged\n"]]]), &keys), [0]);
    assert_eq!(found_keys(&index, &plan(&[&[&["word1\n"]]]), &keys).len(), 4, "word1 of segments 1-4 (doc 1 of segment 0 is gone)");
    // Merge everything once a fifth are tombstones.
    for k in &keys[..80] {
        index.remove(*k);
    }
    while let Some(job) = index.plan_merge(true) {
        for f in index.run_merge(job).unwrap() {
            fs::remove_file(f).unwrap();
        }
    }
    assert_eq!(index.segments(), 1);
    assert_eq!(found_keys(&index, &p, &keys).len(), 120);
    let st = index.lock();
    assert_eq!(st.segments[0].docs, 120, "only the documents left (the changed one was removed too)");
}

#[test]
fn damaged_files_start_the_index_afresh() {
    let dir = temp_dir("damage");
    let index = Index::open(&dir, std::iter::empty());
    let keys = fill(&index);
    index.write_pending(true).unwrap();
    save(&dir, &index, &keys);
    let seg = index.lock().segments[0].path.clone();
    drop(index);
    // A garbled manifest: an empty index, and the segment file goes.
    let man = dir.join(MANIFEST);
    let good = fs::read(&man).unwrap();
    let mut bad = good.clone();
    bad[10] ^= 0xff;
    fs::write(&man, &bad).unwrap();
    let index = Index::open(&dir, stamps(&keys).into_iter());
    assert_eq!(index.stats().docs, 0);
    assert!(!seg.exists());
    drop(index);

    // A segment damaged inside a block: found at the first lookup.
    let index = Index::open(&dir, std::iter::empty());
    let keys = fill(&index);
    index.write_pending(true).unwrap();
    save(&dir, &index, &keys);
    let seg = index.lock().segments[0].path.clone();
    drop(index);
    let mut bytes = fs::read(&seg).unwrap();
    bytes[3] ^= 0x55;
    fs::write(&seg, &bytes).unwrap();
    let index = Index::open(&dir, stamps(&keys).into_iter());
    assert!(matches!(index.candidates(&plan(&[&[&["port"]]])), Found::Unavailable));
    assert!(index.is_broken());
    for f in index.reset() {
        fs::remove_file(f).unwrap();
    }
    assert!(!index.is_broken());
    assert_eq!(index.stats().docs, 0);
    // A cut segment (trailer gone): an empty index at the next start.
    let keys = fill(&index);
    index.write_pending(true).unwrap();
    save(&dir, &index, &keys);
    let seg = index.lock().segments[0].path.clone();
    drop(index);
    let len = fs::metadata(&seg).unwrap().len();
    File::options().write(true).open(&seg).unwrap().set_len(len - 5).unwrap();
    let index = Index::open(&dir, stamps(&keys).into_iter());
    assert_eq!(index.stats().docs, 0);
}

#[test]
fn random_bytes_never_panic() {
    let dir = temp_dir("random");
    let mut rng = Rng(12345);
    let index = Index::open(&dir, std::iter::empty());
    let keys = fill(&index);
    index.write_pending(true).unwrap();
    save(&dir, &index, &keys);
    let seg = index.lock().segments[0].path.clone();
    drop(index);
    let good_seg = fs::read(&seg).unwrap();
    let good_man = fs::read(dir.join(MANIFEST)).unwrap();
    for _ in 0..300 {
        let mut s = good_seg.clone();
        for _ in 0..1 + rng.below(4) {
            let i = rng.below(s.len());
            s[i] = rng.next() as u8;
        }
        fs::write(&seg, &s).unwrap();
        fs::write(dir.join(MANIFEST), &good_man).unwrap();
        let index = Index::open(&dir, stamps(&keys).into_iter());
        let _ = index.candidates(&plan(&[&[&["port"]], &[&["\nريس\n"]]]));
        drop(index);
    }
}

#[test]
#[ignore]
fn bench_doc_words() {
    let mut rng = Rng(99);
    const LETTERS: &[char] = &['ا', 'ب', 'ت', 'ر', 'س', 'ل', 'م', 'ن', 'ه', 'و', 'ي', 'ة', 'أ', 'ى', 'a', 'e', 'r', 's', 't', 'o', '1'];
    let words: Vec<String> = (0..50_000).map(|_| (0..2 + rng.below(8)).map(|_| LETTERS[rng.below(LETTERS.len())]).collect()).collect();
    let text: String = (0..1_000_000).map(|_| words[rng.below(words.len())].as_str()).collect::<Vec<_>>().join(" ");
    let t = std::time::Instant::now();
    let w = doc_words(&text);
    let ms = t.elapsed().as_millis().max(1);
    println!("doc_words: {} MB in {ms} ms ({} MB/s), {} words", text.len() >> 20, (text.len() as u128 / 1000) / ms, w.words().len());
    let t = std::time::Instant::now();
    let n = normalize(&text);
    println!("normalize: {} ms", t.elapsed().as_millis());
    drop(n);
    let small: Vec<String> = text.split(' ').collect::<Vec<_>>().chunks(300).take(3000).map(|c| c.join(" ")).collect();
    let t = std::time::Instant::now();
    let total: usize = small.iter().map(|s| doc_words(s).list.len()).sum();
    println!("3000 small docs: {} ms ({total})", t.elapsed().as_millis());
}

/// What [`crate::normalize::VERSION`] promises: the same normalisation as
/// when the word index was built. A change to `normalize` changes this
/// fingerprint; then bump `normalize::VERSION` (the word index is built
/// again from the stored texts) and put the new fingerprint here.
#[test]
fn normalize_version_is_bumped_with_its_output() {
    let mut rng = Rng(0xfeed_f00d);
    let mut sample = String::new();
    for _ in 0..4000 {
        let len = rng.below(12);
        sample.push_str(&random_text(&mut rng, len));
    }
    // Every Unicode scalar up to the supplementary planes' CJK, one by one.
    let all: String = (0..0x3_0000u32).filter_map(char::from_u32).collect();
    let print = fnv(normalize(&sample).as_bytes()) ^ fnv(normalize(&all).as_bytes()).rotate_left(1);
    assert_eq!(
        (crate::normalize::VERSION, print),
        (1, FINGERPRINT),
        "normalize changed: bump normalize::VERSION and set FINGERPRINT to {print:#x}"
    );
}

const FINGERPRINT: u64 = 0xcd1d_f56a_a91d_5c20;

#[test]
fn scripts_without_spaces_pass_whole_word_pieces() {
    let dir = temp_dir("unspaced");
    let index = Index::open(&dir, std::iter::empty());
    let keys: Vec<u64> = ["東京タワーabc", "plain abc", "xabc"]
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let k = key(&format!("C:\\u{i}.txt"));
            index.replace(k, doc_words(t));
            k
        })
        .collect();
    assert!(doc_words("東京タワーabc").unspaced && !doc_words("plain abc").unspaced);
    let mut p = Plan::default();
    let piece = p.piece(vec![b"\nabc\n".to_vec()], true);
    p.conds.push(vec![vec![piece]]);
    for round in 0..2 {
        // "abc" is a whole word after カ (ww: there); not inside "xabc".
        assert_eq!(found_keys(&index, &p, &keys), [0, 1], "round {round}");
        index.write_pending(true).unwrap();
    }
    // A piece from a real separator stays exact.
    assert_eq!(found_keys(&index, &plan(&[&[&["\nabc\n"]]]), &keys), [1]);
}

#[test]
fn a_write_from_before_a_reset_leaves_the_newer_batch_alone() {
    let dir = temp_dir("reset_race");
    let index = Index::open(&dir, std::iter::empty());
    let _ = fill(&index);
    // A write starts, the index is reset (after damage), the documents are
    // indexed again and a second write starts, then the first one ends.
    let first = index.take_pending(true).unwrap();
    let written = write_pending_segment(&index.dir, first.serial, first.min, first.end, &first.docs, &first.live);
    for f in index.reset() {
        let _ = fs::remove_file(f);
    }
    let keys = fill(&index);
    let second = index.take_pending(true).unwrap();
    let stale = written.as_ref().unwrap().as_ref().unwrap().path.clone();
    assert!(!index.finish_pending(first, written).unwrap());
    // The second batch is still answered from memory; the stale segment
    // is neither used nor kept.
    assert!(index.lock().writing.is_some());
    assert_eq!(index.segments(), 0);
    assert!(!stale.exists());
    check_lookups(&index, &keys);
    // The second write finishes as usual.
    let written = write_pending_segment(&index.dir, second.serial, second.min, second.end, &second.docs, &second.live);
    assert!(index.finish_pending(second, written).unwrap());
    assert!(index.lock().writing.is_none());
    assert_eq!(index.segments(), 1);
    check_lookups(&index, &keys);
}
