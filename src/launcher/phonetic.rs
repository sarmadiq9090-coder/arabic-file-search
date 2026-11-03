//! Sound-alike matching between Arabic spellings and Latin program names:
//! "كوكل بلي" finds "Google Play Games", "تيليجرام" finds "Telegram", for
//! any program, not only those in the [`super::aliases`] table.
//!
//! Both scripts are reduced to a skeleton of consonant classes: vowels and
//! the letters Arabic writers use for them (ا و ي ه ع) are dropped, letters
//! people swap when writing a foreign name in Arabic share a class (g, k, q,
//! j, c and ك ق غ ج خ; p, b and ب; v, f and ف; s, z and س ص ش ز; t, d and
//! ت د ط ض ث ذ ظ), and a doubled class counts once. "google" and "كوكل" are
//! both `KL`; "play" and "بلي" are both `BL`.

/// The class of a Latin letter, `None` for a letter that is left out.
fn latin(c: char) -> Option<char> {
    Some(match c {
        'b' | 'p' => 'B',
        'c' | 'g' | 'j' | 'k' | 'q' => 'K',
        'd' | 't' => 'T',
        'f' | 'v' => 'F',
        's' | 'z' => 'S',
        'l' => 'L',
        'm' => 'M',
        'n' => 'N',
        'r' => 'R',
        'x' => 'X',
        _ => return None,
    })
}

/// The class of an Arabic letter, `None` for a letter that is left out.
fn arabic(c: char) -> Option<char> {
    Some(match c {
        'ب' | 'پ' => 'B',
        'ك' | 'ق' | 'غ' | 'ج' | 'خ' | 'گ' | 'چ' => 'K',
        'ت' | 'د' | 'ط' | 'ض' | 'ث' | 'ذ' | 'ظ' => 'T',
        'ف' | 'ڤ' => 'F',
        'س' | 'ص' | 'ش' | 'ز' => 'S',
        'ل' => 'L',
        'م' => 'M',
        'ن' => 'N',
        'ر' => 'R',
        _ => return None,
    })
}

fn is_arabic(c: char) -> bool {
    ('\u{0600}'..='\u{06FF}').contains(&c)
}

/// Whether `s` has an Arabic letter.
pub fn has_arabic(s: &str) -> bool {
    s.chars().any(is_arabic)
}

/// The skeleton of one word (already normalised: lower case). Latin "ph"
/// is F, "x" is K and S, and "c" before e, i or y is S.
pub fn skeleton(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    let mut out = String::new();
    let push = |k: char, out: &mut String| {
        if !out.ends_with(k) {
            out.push(k);
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == 'p' && chars.get(i + 1) == Some(&'h') {
            push('F', &mut out);
            i += 2;
            continue;
        }
        // A soft c: "office" is أوفيس.
        let soft_c = c == 'c' && matches!(chars.get(i + 1), Some('e' | 'i' | 'y'));
        let class = if soft_c {
            Some('S')
        } else if is_arabic(c) {
            arabic(c)
        } else {
            latin(c)
        };
        match class {
            Some('X') => {
                push('K', &mut out);
                push('S', &mut out);
            }
            Some(k) => push(k, &mut out),
            None if c.is_ascii_digit() => push(c, &mut out),
            None => {}
        }
        i += 1;
    }
    out
}

/// The skeletons of the Latin words of a name ("Google Play Games" →
/// `KL BL KMS`); Arabic words are matched as they are elsewhere.
pub fn name_sounds(name: &str) -> Vec<String> {
    let lower = name.to_lowercase();
    let mut out: Vec<String> = Vec::new();
    for w in lower.split(|c: char| !c.is_alphanumeric()) {
        if w.is_empty() || has_arabic(w) {
            continue;
        }
        let s = skeleton(w);
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    // The whole name run together, for a name written as one word in Latin
    // and as two in Arabic ("CapCut", "كاب كات").
    let all = skeleton(&lower.chars().filter(|c| c.is_alphanumeric() && !is_arabic(*c)).collect::<String>());
    if !all.is_empty() && !out.contains(&all) {
        out.push(all);
    }
    out
}

/// The skeletons of a query's words, when it is worth matching by sound:
/// it has Arabic in it and its words have consonants.
pub fn query_sounds(normalised: &str) -> Option<Vec<String>> {
    if !has_arabic(normalised) {
        return None;
    }
    let words: Vec<String> = normalised.split_whitespace().map(skeleton).collect();
    if words.is_empty() || words.iter().any(String::is_empty) {
        return None;
    }
    // One short word ("بي") would match too much.
    if words.len() == 1 && words[0].chars().count() < 2 {
        return None;
    }
    Some(words)
}

/// Whether every query skeleton is a word of the name: equal, or for the
/// last word (still being typed) the start of one, with three classes at
/// least (fewer would match too many names).
pub fn matches(query: &[String], name: &[String]) -> bool {
    if query.is_empty() || name.is_empty() {
        return false;
    }
    let last = query.len() - 1;
    let word_by_word = query.iter().enumerate().all(|(i, q)| {
        name.iter().any(|n| n == q || (i == last && q.chars().count() >= 3 && n.starts_with(q.as_str())))
    });
    if word_by_word || query.len() == 1 {
        return word_by_word;
    }
    // "كاب كات": the words together start a word of the name.
    let mut joined = String::new();
    for c in query.concat().chars() {
        if !joined.ends_with(c) {
            joined.push(c);
        }
    }
    joined.chars().count() >= 3 && name.iter().any(|n| n.starts_with(joined.as_str()))
}

/// Whether the query is the name said in Arabic, word for word, with two
/// words at least ("جوجل بلاي" for "Google Play Games"): worth more than a
/// web search that starts with the same word.
pub fn is_whole_words(query: &[String], name: &[String]) -> bool {
    query.len() >= 2 && query.iter().all(|q| name.contains(q))
}

/// Whether the query is the name's words themselves, by sound, with four
/// classes at least in all ("نتفلكس" for "Netflix", not "كل" for "Google"):
/// a match as good as a whole word of the name.
pub fn is_exact_words(query: &[String], name: &[String]) -> bool {
    query.iter().map(|q| q.chars().count()).sum::<usize>() >= 4 && query.iter().all(|q| name.contains(q))
}

/// Longest skeleton compared with one edit allowed.
const MAX_TOLERANT: usize = 32;

/// Edits (insert, delete, change, swap two neighbours) between `q` and `n`,
/// or between `q` and the closest start of `n` when `prefix`; 2 stands for
/// "more than one".
fn edits(q: &[u8], n: &[u8], prefix: bool) -> usize {
    if q.len() > MAX_TOLERANT || n.len() > MAX_TOLERANT || (!prefix && q.len().abs_diff(n.len()) > 1) {
        return 2;
    }
    let n = if prefix { &n[..n.len().min(q.len() + 1)] } else { n };
    let w = n.len();
    let mut prev2 = [0usize; MAX_TOLERANT + 2];
    let mut prev = [0usize; MAX_TOLERANT + 2];
    let mut cur = [0usize; MAX_TOLERANT + 2];
    for (j, d) in prev.iter_mut().enumerate().take(w + 1) {
        *d = j;
    }
    for i in 1..=q.len() {
        cur[0] = i;
        for j in 1..=w {
            let cost = usize::from(q[i - 1] != n[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && q[i - 1] == n[j - 2] && q[i - 2] == n[j - 1] {
                d = d.min(prev2[j - 2] + 1);
            }
            cur[j] = d;
        }
        prev2 = prev;
        prev = cur;
    }
    let best = if prefix { prev[..=w].iter().copied().min().unwrap_or(2) } else { prev[w] };
    best.min(2)
}

/// Whether `q` is one edit from a word of the name (or, for the word still
/// being typed, from the start of one), keeping the first class. Short
/// skeletons match too much with any edit: four classes may only miss one
/// ("ديسكود" for discord), five or more may have any one edit.
fn near(q: &str, name: &[String], last: bool) -> bool {
    let qb = q.as_bytes();
    if qb.len() < 4 {
        return false;
    }
    name.iter().any(|n| {
        let nb = n.as_bytes();
        if nb.first() != qb.first() {
            false
        } else if qb.len() == 4 {
            nb.len() == 5 && edits(qb, nb, false) == 1
        } else {
            edits(qb, nb, false) <= 1 || (last && edits(qb, nb, true) <= 1)
        }
    })
}

/// Like [`matches`], with one slip allowed: one query word (or the words run
/// together) one class away from a word of the name ("ديسكود" for
/// "Discord", "نتفليس" for "Netflix"). Scored below the exact matches.
pub fn matches_tolerant(query: &[String], name: &[String]) -> bool {
    if query.is_empty() || name.is_empty() {
        return false;
    }
    let last = query.len() - 1;
    let mut slips = 0;
    for (i, q) in query.iter().enumerate() {
        let exact = name.iter().any(|n| n == q || (i == last && q.chars().count() >= 3 && n.starts_with(q.as_str())));
        if exact {
            continue;
        }
        if slips == 0 && near(q, name, i == last) {
            slips = 1;
            continue;
        }
        slips = 2;
        break;
    }
    if slips <= 1 {
        return true;
    }
    if query.len() == 1 {
        return false;
    }
    let mut joined = String::new();
    for c in query.concat().chars() {
        if !joined.ends_with(c) {
            joined.push(c);
        }
    }
    near(&joined, name, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::normalize;

    fn finds(query: &str, name: &str) -> bool {
        query_sounds(&normalize(query)).is_some_and(|q| matches(&q, &name_sounds(name)))
    }

    #[test]
    fn arabic_spellings_find_latin_names() {
        assert!(finds("كوكل بلي", "Google Play Games"));
        assert!(finds("جوجل بلاي", "Google Play Games"));
        assert!(finds("قوقل بلاي قيمز", "Google Play Games"));
        assert!(finds("غوغل", "Google Chrome"));
        assert!(finds("تيليجرام", "Telegram Desktop"));
        assert!(finds("نتفلكس", "Netflix"));
        assert!(finds("سبوتيفاي", "Spotify"));
        assert!(finds("ستيم", "Steam"));
        assert!(finds("فوتوشوب", "Adobe Photoshop 2024"));
        assert!(finds("بلوتوث", "Bluetooth"));
        assert!(finds("واتساب", "WhatsApp"));
        assert!(finds("زوم", "Zoom Workplace"));
        assert!(finds("اوفيس", "Microsoft Office"));
        assert!(finds("كاب كات", "CapCut"));
        assert!(finds("واتس اب", "WhatsApp"));
        assert!(finds("ون درايف", "OneDrive"));
        // Still typing the last word.
        assert!(finds("جوجل بل", "Google Play Games"));
    }

    #[test]
    fn unrelated_names_do_not_match() {
        assert!(!finds("كوكل بلي", "Microsoft Edge"));
        assert!(!finds("نتفلكس", "Notepad"));
        assert!(!finds("ب", "Bluetooth"));
        // Latin-only queries are matched elsewhere.
        assert!(query_sounds("google").is_none());
    }

    fn finds_tolerant(query: &str, name: &str) -> bool {
        query_sounds(&normalize(query)).is_some_and(|q| matches_tolerant(&q, &name_sounds(name)))
    }

    #[test]
    fn one_slip_is_tolerated() {
        assert!(!finds("ديسكود", "Discord") && finds_tolerant("ديسكود", "Discord"));
        assert!(!finds("نتفليس", "Netflix") && finds_tolerant("نتفليس", "Netflix"));
        assert!(finds_tolerant("سبوتفاي", "Spotify"));
        assert!(finds_tolerant("فوتشوب", "Adobe Photoshop"));
        assert!(finds_tolerant("تلكرم", "Telegram Desktop"));
        assert!(!finds("انسكرام", "Instagram") && finds_tolerant("انسكرام", "Instagram"));
        // A slip in a word still being typed: near the start of a longer word.
        let long = ["FTSBKSM".to_string()];
        assert!(near("FTXBK", &long, true) && !near("FTXBK", &long, false));
        // Four classes: only one missing, and the first class kept.
        assert!(near("TSKT", &["TSKRT".into()], false) && !near("TSKR", &["TSKT".into()], false));
        assert!(!near("XSKRT", &["TSKRT".into()], false));
        // Common Arabic words that are one class off a name do not match.
        assert!(!finds_tolerant("مذكرة", "Task Manager"));
        assert!(!finds_tolerant("درجات", "Docker Desktop"));
        assert!(!finds_tolerant("مونتاج", "Minecraft Launcher"));
        // Short skeletons get no slip.
        assert!(!finds_tolerant("زوم", "Steam"));
        assert!(!finds_tolerant("كل", "Zoom"));
        assert!(!finds_tolerant("نتفلكس", "Notepad"));
        assert!(!finds_tolerant("ديسكود", "Microsoft Edge"));
        assert_eq!(edits(b"TSKT", b"TSKRT", false), 1);
        assert_eq!(edits(b"TSKRT", b"TSKRT", false), 0);
        assert_eq!(edits(b"FTSB", b"FTSBKS", true), 0);
        assert_eq!(edits(b"ABCD", b"WXYZ", false), 2);
        assert!(is_exact_words(&["NTFLKS".into()], &name_sounds("Netflix")));
        assert!(!is_exact_words(&["KL".into()], &name_sounds("Google Chrome")));
    }

    #[test]
    fn skeletons() {
        assert_eq!(skeleton("google"), "KL");
        assert_eq!(skeleton("كوكل"), "KL");
        assert_eq!(skeleton("play"), "BL");
        assert_eq!(skeleton("بلي"), "BL");
        assert_eq!(skeleton("photoshop"), "FTSB");
        assert_eq!(skeleton("فوتوشوب"), "FTSB");
        assert_eq!(skeleton("xbox"), "KSBKS");
        assert_eq!(skeleton("اكس بوكس"), "KSBKS");
        assert_eq!(name_sounds("Google Play Games"), vec!["KL", "BL", "KMS", "KLBLKMS"]);
        assert_eq!(name_sounds("Office 365"), vec!["FS", "365", "FS365"]);
    }
}
