//! Currency conversion for the instant answers: "100 usd to iqd",
//! "100 دولار", "50 يورو بالدينار", "100$ to iqd", "١٠٠ دولار عراقي".
//!
//! The rates come from a table compiled into the program (approximate, so
//! the answer works offline from the first run) and are replaced by fresher
//! ones from <https://open.er-api.com> (free, no key), fetched in the
//! background at most once every [`REFRESH_SECS`] and cached in
//! `rates.json` next to the index cache. Downloaded and cached data is
//! checked strictly; anything odd is ignored and the bundled table stays.
//!
//! The rates in use live in one process-wide slot ([`current`]); the query
//! side only reads it, the refresh thread swaps in a new table.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use super::calc;
use crate::i18n::Lang;

/// Free exchange-rate API (no key), rates per US dollar.
#[cfg_attr(not(windows), allow(dead_code))]
pub const API_URL: &str = "https://open.er-api.com/v6/latest/USD";
/// How old the cached rates may get before a new download is tried.
pub const REFRESH_SECS: i64 = 12 * 60 * 60;
/// After a failed download, wait this long before trying again.
#[cfg_attr(not(windows), allow(dead_code))]
pub const RETRY_SECS: i64 = 60 * 60;
/// Upper bound for the downloaded JSON and the cache file (the real answer is about 3 KB).
pub const MAX_JSON: usize = 256 * 1024;
/// When the bundled table was written (2026-10-01, UTC).
const BUNDLED_UPDATED: i64 = 1_790_812_800;
/// Rates before this (2020-01-01) are not believable.
const OLDEST_BELIEVABLE: i64 = 1_577_836_800;
/// A downloaded rate more than this factor away from the bundled one is
/// treated as broken data (and the bundled rate is kept for that currency).
const SANE_FACTOR: f64 = 50.0;
/// Band for the other currencies, wide enough for a redenomination (÷1000).
const SANE_FACTOR_WIDE: f64 = 2000.0;
/// Largest amount accepted in a query.
const MAX_AMOUNT: f64 = 1e12;

struct Currency {
    code: &'static str,
    /// Name shown in the Arabic interface.
    ar: &'static str,
    /// Units per US dollar in the bundled table.
    bundled: f64,
    /// Names, codes and symbols, in any case and spelling (normalised when the table is built).
    aliases: &'static [&'static str],
    /// Words that only name the currency after another one ("100 دولار عراقي"
    /// = 100 dollars in Iraqi dinars); alone they are too vague to start a query.
    target_only: &'static [&'static str],
}

const fn c(
    code: &'static str,
    ar: &'static str,
    bundled: f64,
    aliases: &'static [&'static str],
    target_only: &'static [&'static str],
) -> Currency {
    Currency { code, ar, bundled, aliases, target_only }
}

/// Index of the US dollar and the Iraqi dinar in [`CURRENCIES`].
const USD: usize = 0;
const IQD: usize = 3;
const EUR: usize = 1;
/// The toman is not in the API: it is ten Iranian rials.
const TOMAN_CODE: &str = "TOMAN";

static CURRENCIES: &[Currency] = &[
    c("USD", "دولار", 1.0, &["usd", "$", "dollar", "dollars", "us dollar", "us dollars", "american dollar", "american dollars", "دولار", "دولارات", "دولارا", "دولار امريكي", "دولارات امريكيه", "دولار أمريكي"], &["امريكي", "american", "us"]),
    c("EUR", "يورو", 0.86, &["eur", "€", "euro", "euros", "يورو", "يوروات"], &["اوروبي"]),
    c("GBP", "جنيه إسترليني", 0.76, &["gbp", "£", "sterling", "pound sterling", "british pound", "british pounds", "استرليني", "جنيه استرليني", "جنيهات استرلينيه", "باوند استرليني", "باوند بريطاني", "جنيه بريطاني"], &["بريطاني", "british"]),
    c("IQD", "دينار عراقي", 1310.0, &["iqd", "dinar", "dinars", "iraqi dinar", "iraqi dinars", "دينار", "دنانير", "دينارا", "دينار عراقي", "د.ع", "ع.د"], &["عراقي", "iraqi"]),
    c("SAR", "ريال سعودي", 3.75, &["sar", "riyal", "riyals", "saudi riyal", "saudi riyals", "ريال", "ريالات", "ريال سعودي", "ر.س"], &["سعودي", "saudi"]),
    c("AED", "درهم إماراتي", 3.6725, &["aed", "dirham", "dirhams", "uae dirham", "uae dirhams", "emirati dirham", "emirati dirhams", "درهم", "دراهم", "درهم اماراتي", "د.إ"], &["اماراتي", "emirati", "uae"]),
    c("KWD", "دينار كويتي", 0.306, &["kwd", "kuwaiti dinar", "kuwaiti dinars", "دينار كويتي", "د.ك"], &["كويتي", "kuwaiti"]),
    c("QAR", "ريال قطري", 3.64, &["qar", "qatari riyal", "qatari riyals", "ريال قطري", "ر.ق"], &["قطري", "qatari"]),
    c("BHD", "دينار بحريني", 0.376, &["bhd", "bahraini dinar", "bahraini dinars", "دينار بحريني"], &["بحريني", "bahraini"]),
    c("OMR", "ريال عماني", 0.3845, &["omr", "omani rial", "omani rials", "ريال عماني", "ر.ع"], &["عماني", "omani"]),
    c("JOD", "دينار أردني", 0.709, &["jod", "jordanian dinar", "jordanian dinars", "دينار اردني"], &["اردني", "jordanian"]),
    c("EGP", "جنيه مصري", 47.5, &["egp", "egyptian pound", "egyptian pounds", "جنيه", "جنيهات", "جنيه مصري", "جنيهات مصريه"], &["مصري", "egyptian"]),
    c("TRY", "ليرة تركية", 42.5, &["try", "₺", "lira", "liras", "turkish lira", "turkish liras", "ليره", "ليرات", "ليره تركيه", "ليره تركي", "ليرات تركيه"], &["تركي", "تركيه", "turkish"]),
    c("IRR", "ريال إيراني", 42_000.0, &["irr", "rial", "rials", "iranian rial", "iranian rials", "ريال ايراني"], &["ايراني", "iranian"]),
    c(TOMAN_CODE, "تومان", 4_200.0, &["toman", "tomans", "تومان", "تومانات"], &[]),
    c("SYP", "ليرة سورية", 13_000.0, &["syp", "syrian pound", "syrian pounds", "syrian lira", "ليره سوريه", "ليره سوري", "ليرات سوريه"], &["سوري", "سوريه", "syrian"]),
    c("LBP", "ليرة لبنانية", 89_500.0, &["lbp", "lebanese pound", "lebanese pounds", "lebanese lira", "ليره لبنانيه", "ليره لبناني", "ليرات لبنانيه"], &["لبناني", "لبنانيه", "lebanese"]),
    c("JPY", "ين ياباني", 152.0, &["jpy", "¥", "yen", "japanese yen", "ين", "ين ياباني"], &["ياباني", "japanese"]),
    c("CNY", "يوان صيني", 7.12, &["cny", "rmb", "yuan", "renminbi", "chinese yuan", "يوان", "يوان صيني"], &["صيني", "chinese"]),
    c("INR", "روبية هندية", 88.5, &["inr", "₹", "rupee", "rupees", "indian rupee", "indian rupees", "روبيه", "روبيات", "روبيه هنديه", "روبيه هندي"], &["هندي", "هنديه", "indian"]),
    c("CAD", "دولار كندي", 1.40, &["cad", "canadian dollar", "canadian dollars", "دولار كندي", "دولارات كنديه"], &["كندي", "canadian"]),
    c("AUD", "دولار أسترالي", 1.53, &["aud", "australian dollar", "australian dollars", "دولار استرالي", "دولارات استراليه"], &["استرالي", "australian"]),
    c("CHF", "فرنك سويسري", 0.80, &["chf", "franc", "francs", "swiss franc", "swiss francs", "فرنك", "فرنكات", "فرنك سويسري"], &["سويسري", "swiss"]),
    c("RUB", "روبل روسي", 81.0, &["rub", "₽", "ruble", "rubles", "rouble", "roubles", "russian ruble", "روبل", "روبلات", "روبل روسي"], &["روسي", "russian"]),
];

/// Words between the amount and the currency asked for ("to", "بال"…).
const CONNECTORS: &[&str] = &["to", "in", "into", "as", "=", ">", "-", "→", "الى", "ل", "ب", "لل", "بال", "كم", "يساوي"];
/// Prefixes glued to an Arabic currency name ("بالدينار", "للدولار", "الين").
const GLUED: &[&str] = &["بال", "لل", "ال", "ب", "ل"];

// ---------------------------------------------------------------- rates

/// A table of exchange rates: units of each currency per US dollar.
#[derive(Clone, Debug, PartialEq)]
pub struct Rates {
    /// When the rates were published (seconds since 1970).
    pub updated: i64,
    /// When they were downloaded (seconds since 1970); 0 for the bundled table.
    pub fetched: i64,
    /// Units per US dollar, in the order of [`CURRENCIES`].
    per_usd: Vec<f64>,
}

impl Rates {
    /// The table compiled into the program.
    pub fn bundled() -> Rates {
        Rates { updated: BUNDLED_UPDATED, fetched: 0, per_usd: CURRENCIES.iter().map(|c| c.bundled).collect() }
    }

    pub fn is_bundled(&self) -> bool {
        self.fetched == 0
    }

    /// Units of `code` per US dollar.
    #[cfg(test)]
    fn rate(&self, code: &str) -> Option<f64> {
        let i = CURRENCIES.iter().position(|c| c.code == code)?;
        self.per_usd.get(i).copied()
    }

    /// Builds a table from a `code → units per USD` map, checking every
    /// value. Currencies missing from the map or with an implausible value
    /// keep the bundled rate; the whole map is refused when the dollar is not
    /// 1, the dates are off, or too few of our currencies are usable.
    fn from_map(map: &serde_json::Map<String, serde_json::Value>, updated: i64, fetched: i64, now: i64) -> Result<Rates, String> {
        if !(OLDEST_BELIEVABLE..=now + 2 * 86_400).contains(&updated) {
            return Err(format!("rates date {updated} is not believable"));
        }
        if !(OLDEST_BELIEVABLE..=now + 2 * 86_400).contains(&fetched) {
            return Err(format!("download time {fetched} is not believable"));
        }
        if map.len() > 1000 {
            return Err("too many rates".into());
        }
        let usd = map.get("USD").and_then(|v| v.as_f64());
        if !usd.is_some_and(|u| (u - 1.0).abs() < 1e-9) {
            return Err("the rates are not per US dollar".into());
        }
        let mut per_usd = Vec::with_capacity(CURRENCIES.len());
        let mut sane = Vec::with_capacity(CURRENCIES.len());
        for cur in CURRENCIES {
            let raw = if cur.code == TOMAN_CODE {
                map.get("IRR").and_then(|v| v.as_f64()).map(|r| r / 10.0)
            } else {
                map.get(cur.code).and_then(|v| v.as_f64())
            };
            // Currencies can be redenominated (a zero or two removed), so
            // the minor ones get a wide band; the dinar and euro a narrow one.
            let factor = if cur.code == "IQD" || cur.code == "EUR" { SANE_FACTOR } else { SANE_FACTOR_WIDE };
            let ok = raw.filter(|&r| r.is_finite() && r > 0.0 && r >= cur.bundled / factor && r <= cur.bundled * factor);
            sane.push(ok.is_some());
            per_usd.push(ok.unwrap_or(cur.bundled));
        }
        // Most of our currencies must be usable, and always the two that
        // matter most here.
        let good = sane.iter().filter(|&&b| b).count();
        if good + 4 < CURRENCIES.len() {
            return Err(format!("only {good} usable rates"));
        }
        if !sane[IQD] || !sane[EUR] {
            return Err("no usable dinar or euro rate".into());
        }
        Ok(Rates { updated, fetched, per_usd })
    }

    fn to_json(&self) -> String {
        let mut rates = serde_json::Map::new();
        rates.insert("USD".into(), 1.0.into());
        for (cur, &r) in CURRENCIES.iter().zip(&self.per_usd) {
            if cur.code != TOMAN_CODE {
                rates.insert(cur.code.into(), r.into());
            }
        }
        serde_json::json!({ "version": 1, "updated": self.updated, "fetched": self.fetched, "rates": rates }).to_string()
    }
}

/// Reads the answer of [`API_URL`], downloaded at `now`.
pub fn parse_api(body: &[u8], now: i64) -> Result<Rates, String> {
    if body.len() > MAX_JSON {
        return Err("the answer is too large".into());
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    if v["result"].as_str() != Some("success") {
        return Err("the rates service reported an error".into());
    }
    if v.get("base_code").is_some_and(|b| b.as_str() != Some("USD")) {
        return Err("the rates are not per US dollar".into());
    }
    let updated = v["time_last_update_unix"].as_i64().ok_or("no update time")?;
    let map = v["rates"].as_object().ok_or("no rates")?;
    Rates::from_map(map, updated, now, now)
}

/// The rates saved by [`save_cache`], if the file is there and sound.
pub fn load_cache(path: &Path, now: i64) -> Option<Rates> {
    if std::fs::metadata(path).ok()?.len() > MAX_JSON as u64 {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if v["version"].as_i64() != Some(1) {
        return None;
    }
    let (updated, fetched) = (v["updated"].as_i64()?, v["fetched"].as_i64()?);
    Rates::from_map(v["rates"].as_object()?, updated, fetched, now).ok()
}

/// Writes `rates` to `path` (through a temporary file, so a crash never
/// leaves half a file).
pub fn save_cache(path: &Path, rates: &Rates) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, rates.to_json())?;
    std::fs::rename(tmp, path)
}

/// True when rates downloaded at `fetched` should be downloaded again at
/// `now` (also when the clock went back past `fetched`).
pub fn is_due(fetched: i64, now: i64) -> bool {
    fetched <= 0 || now < fetched || now - fetched >= REFRESH_SECS
}

/// One refresh step, without the network or the global slot so that it can
/// be tested: hands the cached rates to `install` (if any), and when they are
/// missing or old, downloads new ones with `fetch`, saves and installs them.
/// Returns whether a download was attempted.
pub fn refresh_with(
    path: &Path,
    now: i64,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
    install: &mut dyn FnMut(Rates),
) -> Result<bool, String> {
    let cached = load_cache(path, now);
    let due = cached.as_ref().is_none_or(|c| is_due(c.fetched, now));
    if let Some(c) = cached {
        install(c);
    }
    if !due {
        return Ok(false);
    }
    let fresh = parse_api(&fetch()?, now)?;
    let _ = save_cache(path, &fresh);
    install(fresh);
    Ok(true)
}

fn slot() -> &'static RwLock<Option<Arc<Rates>>> {
    static SLOT: RwLock<Option<Arc<Rates>>> = RwLock::new(None);
    &SLOT
}

fn bundled_arc() -> Arc<Rates> {
    static BUNDLED: OnceLock<Arc<Rates>> = OnceLock::new();
    BUNDLED.get_or_init(|| Arc::new(Rates::bundled())).clone()
}

/// The rates the answers use now: the newest downloaded ones, or the bundled table.
pub fn current() -> Arc<Rates> {
    slot().read().ok().and_then(|g| g.clone()).unwrap_or_else(bundled_arc)
}

/// Makes `rates` the ones in use, unless they are older than those already in use.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn install(rates: Rates) {
    if rates.updated < current().updated {
        return;
    }
    if let Ok(mut g) = slot().write() {
        *g = Some(Arc::new(rates));
    }
}

/// Loads the cached rates and downloads new ones when they are over
/// [`REFRESH_SECS`] old. Blocks (network); call it from a background
/// thread. Only one runs at a time, and a failed download is not retried
/// for [`RETRY_SECS`]. Returns a line for the log when something happened.
#[cfg(windows)]
pub fn refresh_if_due() -> Option<String> {
    use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
    static BUSY: AtomicBool = AtomicBool::new(false);
    static LAST_FAILURE: AtomicI64 = AtomicI64::new(0);
    if BUSY.swap(true, Ordering::AcqRel) {
        return None;
    }
    let now = crate::index::now_secs();
    let path = crate::index::cache_path().with_file_name("rates.json");
    let failed_at = LAST_FAILURE.load(Ordering::Relaxed);
    let retry_ok = failed_at == 0 || now < failed_at || now - failed_at >= RETRY_SECS;
    let result = crate::crash_log::catch(|| {
        let fetch = || {
            if retry_ok { crate::update::fetch_json(API_URL, MAX_JSON as u64) } else { Err("waiting before retrying".into()) }
        };
        refresh_with(&path, now, fetch, &mut install)
    });
    let line = match result {
        Some(Ok(true)) => {
            LAST_FAILURE.store(0, Ordering::Relaxed);
            Some(format!("exchange rates: updated (rates of {})", date_text(current().updated)))
        }
        Some(Ok(false)) => None,
        Some(Err(e)) if retry_ok => {
            LAST_FAILURE.store(now, Ordering::Relaxed);
            Some(format!("exchange rates: refresh failed: {e}"))
        }
        Some(Err(_)) => None,
        None => {
            LAST_FAILURE.store(now, Ordering::Relaxed);
            Some("exchange rates: refresh panicked".to_string())
        }
    };
    BUSY.store(false, Ordering::Release);
    line
}

// ---------------------------------------------------------------- queries

/// A parsed conversion: `amount` of currency `from` into `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Query {
    amount: f64,
    from: usize,
    /// `None`: the user named no target; a default is picked.
    to: Option<usize>,
}

/// Arabic letter variants folded together (أ إ آ → ا, ة → ه, ى → ي), tatweel
/// and direction marks dropped, Latin letters lower-cased.
fn fold(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            'أ' | 'إ' | 'آ' | 'ٱ' => Some('ا'),
            'ة' => Some('ه'),
            'ى' => Some('ي'),
            'ـ' | '\u{064B}'..='\u{0652}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => None,
            _ => Some(c.to_lowercase().next().unwrap_or(c)),
        })
        .collect()
}

/// Alias → (currency, only as a target).
fn alias_table() -> &'static HashMap<String, (usize, bool)> {
    static TABLE: OnceLock<HashMap<String, (usize, bool)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = HashMap::new();
        for (i, cur) in CURRENCIES.iter().enumerate() {
            for a in cur.aliases {
                t.insert(fold(a), (i, false));
            }
            for a in cur.target_only {
                t.entry(fold(a)).or_insert((i, true));
            }
        }
        t
    })
}

fn is_symbol(c: char) -> bool {
    matches!(c, '$' | '€' | '£' | '¥' | '₺' | '₹' | '₽' | '=' | '>' | '→')
}

/// Splits folded text into numbers, words and symbols: "100$" → "100", "$".
fn tokens(s: &str) -> Vec<String> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Num,
        Word,
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut class = Class::Word;
    for c in s.chars() {
        if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if is_symbol(c) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
            continue;
        }
        // "." and "," belong to a number only inside one ("1,000.5"); in a
        // word they stay ("د.ع").
        let this = if c.is_ascii_digit() || (matches!(c, '.' | ',') && class == Class::Num && !cur.is_empty()) {
            Class::Num
        } else {
            Class::Word
        };
        if !cur.is_empty() && this != class {
            out.push(std::mem::take(&mut cur));
        }
        class = this;
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The amount in one token, if the whole token is a number.
fn amount(tok: &str) -> Option<f64> {
    let chars: Vec<char> = tok.chars().collect();
    let mut pos = 0;
    let v = calc::read_number(&chars, &mut pos)??;
    (pos == chars.len() && v > 0.0 && v <= MAX_AMOUNT).then_some(v)
}

/// Looks up a phrase of one to three words; Arabic words may carry a glued
/// "بال"/"لل"/"ال"/"ب"/"ل" at the start, and the following words an "ال".
fn lookup(words: &[String], target: bool) -> Option<usize> {
    let t = alias_table();
    let hit = |key: &str| t.get(key).filter(|&&(_, only)| target || !only).map(|&(i, _)| i);
    let rest: Vec<&str> = words[1..]
        .iter()
        .map(|w| match w.strip_prefix("ال") {
            Some(r) if r.chars().count() >= 2 => r,
            _ => w.as_str(),
        })
        .collect();
    let with = |first: &str| {
        let mut k = first.to_string();
        for r in &rest {
            k.push(' ');
            k.push_str(r);
        }
        k
    };
    let first = words[0].as_str();
    if let Some(i) = hit(&with(first)) {
        return Some(i);
    }
    GLUED.iter().find_map(|p| {
        let r = first.strip_prefix(p)?;
        if r.chars().count() < 2 { None } else { hit(&with(r)) }
    })
}

/// The longest currency phrase at `toks[pos..]`: (currency, words used).
fn phrase(toks: &[String], pos: usize, target: bool) -> Option<(usize, usize)> {
    (1..=3).rev().filter(|n| pos + n <= toks.len()).find_map(|n| lookup(&toks[pos..pos + n], target).map(|i| (i, n)))
}

fn parse(query: &str) -> Option<Query> {
    if query.chars().count() > 80 {
        return None;
    }
    let toks = tokens(&fold(&calc::to_ascii(query)));
    if toks.len() < 2 || toks.len() > 9 {
        return None;
    }
    let mut pos = 0;
    // "$100".
    let mut from = None;
    if toks[0].chars().all(is_symbol) {
        from = Some(lookup(&toks[..1], false)?);
        pos = 1;
    }
    let amount = amount(toks.get(pos)?)?;
    pos += 1;
    let from = match from {
        Some(f) => f,
        None => {
            let (f, n) = phrase(&toks, pos, false)?;
            pos += n;
            f
        }
    };
    let mut connectors = 0;
    while pos < toks.len() && CONNECTORS.iter().any(|c| fold(c) == toks[pos]) {
        pos += 1;
        connectors += 1;
    }
    if pos == toks.len() {
        return (connectors == 0).then_some(Query { amount, from, to: None });
    }
    let (to, n) = phrase(&toks, pos, true)?;
    (pos + n == toks.len()).then_some(Query { amount, from, to: Some(to) })
}

/// "2026-11-02" for a time in seconds since 1970 (UTC).
pub fn date_text(secs: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A money amount: no decimals from 1,000 up, two from 1, and three
/// significant digits below 1; thousands grouped ("131,000", "0.763").
fn format_money(v: f64) -> Option<String> {
    if !v.is_finite() || v <= 0.0 || v >= 1e15 {
        return None;
    }
    let decimals = if v >= 1000.0 {
        0
    } else if v >= 1.0 {
        2
    } else {
        ((-v.log10()).floor() as i64 + 3).clamp(2, 10) as usize
    };
    let s = format!("{v:.decimals$}");
    let (int_part, frac) = match s.split_once('.') {
        Some((i, f)) => (i, f.trim_end_matches('0')),
        None => (s.as_str(), ""),
    };
    if int_part.bytes().all(|b| b == b'0') && frac.is_empty() {
        return None;
    }
    let mut out = calc::group_thousands(int_part);
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    Some(out)
}

fn name(i: usize, english: bool) -> &'static str {
    let cur = &CURRENCIES[i];
    // In English the ISO code (the same in every language), but a word for the toman, which has none.
    match (english, cur.code == TOMAN_CODE) {
        (true, true) => Lang::En.pick("تومان", "toman"),
        (true, false) => cur.code,
        (false, _) => cur.ar,
    }
}

fn convert(amount: f64, from: usize, to: usize, rates: &Rates) -> f64 {
    amount / rates.per_usd[from] * rates.per_usd[to]
}

/// The answer for a currency query: (title, text to copy, note). The note
/// gives the date of the rates (and a second conversion for the dollar).
pub fn answer(query: &str, english: bool, rates: &Rates) -> Option<(String, String, String)> {
    let q = parse(query)?;
    let digits = calc::detect_digits(query);
    let to = q.to.unwrap_or(if q.from == USD { IQD } else { USD });
    if to == q.from {
        return None;
    }
    let result = format_money(convert(q.amount, q.from, to, rates))?;
    let loc = |s: &str| calc::localize(s, digits);
    let shown = calc::format_number(q.amount, 12);
    let title = loc(&format!("{shown} {} = {result} {}", name(q.from, english), name(to, english)));
    let copy = loc(&result.replace(',', ""));
    let mut note = Vec::new();
    // "100 دولار" alone: the dinar first, then the euro too.
    if q.to.is_none()
        && q.from == USD
        && let Some(eur) = format_money(convert(q.amount, USD, EUR, rates))
    {
        note.push(loc(&format!("= {eur} {}", name(EUR, english))));
    }
    let date = loc(&date_text(rates.updated));
    let lang = Lang::from_english(english);
    note.push(if rates.is_bundled() {
        trf!(in lang; "سعر تقريبي مضمّن · {date}", "approximate built-in rate · {date}")
    } else {
        trf!(in lang; "سعر تقريبي · {date}", "approximate rate · {date}")
    });
    Some((title, copy, note.join(" · ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(q: &str) -> Option<(&'static str, &'static str, f64)> {
        let p = parse(q)?;
        let to = p.to.unwrap_or(if p.from == USD { IQD } else { USD });
        Some((CURRENCIES[p.from].code, CURRENCIES[to].code, p.amount))
    }

    fn ans(q: &str, english: bool) -> Option<(String, String, String)> {
        answer(q, english, &Rates::bundled())
    }

    #[test]
    fn alias_table_is_sane() {
        let mut seen: HashMap<String, &str> = HashMap::new();
        for cur in CURRENCIES {
            assert!(cur.bundled.is_finite() && cur.bundled > 0.0, "{}", cur.code);
            for a in cur.aliases.iter().chain(cur.target_only) {
                let k = fold(a);
                if let Some(other) = seen.insert(k.clone(), cur.code) {
                    assert_eq!(other, cur.code, "alias {k:?} used twice");
                }
                // Every alias is reachable through the tokenizer.
                let q = format!("5 {a}");
                if cur.aliases.contains(a) {
                    assert_eq!(conv(&q).map(|c| c.0), Some(cur.code), "{q}");
                }
            }
        }
        assert_eq!(CURRENCIES[USD].code, "USD");
        assert_eq!(CURRENCIES[EUR].code, "EUR");
        assert_eq!(CURRENCIES[IQD].code, "IQD");
        let irr = Rates::bundled().rate("IRR").unwrap();
        assert_eq!(Rates::bundled().rate(TOMAN_CODE), Some(irr / 10.0));
    }

    #[test]
    fn english_queries() {
        assert_eq!(conv("100 usd to iqd"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("100 USD in IQD"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("100 dollar"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("100 dollars"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("100$ to iqd"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("$100"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("$ 100 to eur"), Some(("USD", "EUR", 100.0)));
        assert_eq!(conv("1000 iqd usd"), Some(("IQD", "USD", 1000.0)));
        assert_eq!(conv("1,000 iqd = usd"), Some(("IQD", "USD", 1000.0)));
        assert_eq!(conv("50 eur -> gbp"), Some(("EUR", "GBP", 50.0)));
        assert_eq!(conv("50€"), Some(("EUR", "USD", 50.0)));
        assert_eq!(conv("2.5 kuwaiti dinars to iraqi dinar"), Some(("KWD", "IQD", 2.5)));
        assert_eq!(conv("100 usd iraqi"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("10 swiss francs in yen"), Some(("CHF", "JPY", 10.0)));
        assert_eq!(conv("100usd"), Some(("USD", "IQD", 100.0)));
    }

    #[test]
    fn arabic_queries() {
        assert_eq!(conv("100 دولار"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("100 دولار عراقي"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("50 يورو بالدينار"), Some(("EUR", "IQD", 50.0)));
        assert_eq!(conv("50 يورو بـالدينار"), Some(("EUR", "IQD", 50.0)));
        assert_eq!(conv("50 يورو الى الدولار"), Some(("EUR", "USD", 50.0)));
        assert_eq!(conv("50 يورو إلى الدولار"), Some(("EUR", "USD", 50.0)));
        assert_eq!(conv("١٠٠ دولار"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("۱۰۰ دلار"), None); // Persian word for dollar is not in the table
        assert_eq!(conv("١٬٠٠٠ دينار"), Some(("IQD", "USD", 1000.0)));
        assert_eq!(conv("2٫5 دينار كويتي"), Some(("KWD", "USD", 2.5)));
        assert_eq!(conv("100 ريال سعودي"), Some(("SAR", "USD", 100.0)));
        assert_eq!(conv("100 ريال"), Some(("SAR", "USD", 100.0)));
        assert_eq!(conv("100 درهم"), Some(("AED", "USD", 100.0)));
        assert_eq!(conv("100 ليرة تركية"), Some(("TRY", "USD", 100.0)));
        assert_eq!(conv("100 ليرة"), Some(("TRY", "USD", 100.0)));
        assert_eq!(conv("100 ليرة سورية"), Some(("SYP", "USD", 100.0)));
        assert_eq!(conv("100 جنيه مصري"), Some(("EGP", "USD", 100.0)));
        assert_eq!(conv("100 جنيه استرليني"), Some(("GBP", "USD", 100.0)));
        assert_eq!(conv("100 دينار كويتي"), Some(("KWD", "USD", 100.0)));
        assert_eq!(conv("100 تومان"), Some(("TOMAN", "USD", 100.0)));
        assert_eq!(conv("100 دولار بالدينار الكويتي"), Some(("USD", "KWD", 100.0)));
        assert_eq!(conv("100 دولار كندي"), Some(("CAD", "USD", 100.0)));
        assert_eq!(conv("100 دولار باليوان"), Some(("USD", "CNY", 100.0)));
        assert_eq!(conv("100 دولار للين"), Some(("USD", "JPY", 100.0)));
        assert_eq!(conv("100 دولار بالليرة"), Some(("USD", "TRY", 100.0)));
        assert_eq!(conv("100 دولار ب الريال القطري"), Some(("USD", "QAR", 100.0)));
        assert_eq!(conv("100 \u{200F}دولار\u{200F}"), Some(("USD", "IQD", 100.0)));
        assert_eq!(conv("1000 د.ع"), Some(("IQD", "USD", 1000.0)));
        assert_eq!(conv("100 الدولار الأمريكي"), Some(("USD", "IQD", 100.0)));
    }

    #[test]
    fn ordinary_searches_are_not_currency() {
        for q in [
            "", "usd", "دولار", "100", "100 files", "report 2024", "2024 report", "2+2", "5 km to miles", "100 usd to",
            "100 usd to files", "100 دولار ملف", "usd 100", "0 usd", "-5 usd", "1.2.3 usd", "100 عراقي", "100 american",
            "100 pounds", "100 pound", "100 usd to usd", "100 دولار بالدولار", "1e5 usd", "100 to iqd", "$", "$ $",
            "100 دولار عراقي سعودي", "99999999999999 usd",
        ] {
            assert_eq!(ans(q, false), None, "{q:?}");
        }
        // Long text is never a currency query.
        assert_eq!(ans(&format!("100 usd {}", "x".repeat(200)), false), None);
    }

    #[test]
    fn answer_text_and_copy() {
        let (t, copy, note) = ans("100 usd to iqd", true).unwrap();
        assert_eq!(t, "100 USD = 131,000 IQD");
        assert_eq!(copy, "131000");
        assert_eq!(note, "approximate built-in rate · 2026-10-01");
        let (t, copy, note) = ans("100 دولار", false).unwrap();
        assert_eq!(t, "100 دولار = 131,000 دينار عراقي");
        assert_eq!(copy, "131000");
        assert_eq!(note, "= 86 يورو · سعر تقريبي مضمّن · 2026-10-01");
        let (t, copy, _) = ans("١٠٠ دولار", false).unwrap();
        assert_eq!(t, "١٠٠ دولار = ١٣١٬٠٠٠ دينار عراقي");
        assert_eq!(copy, "١٣١٠٠٠");
        let (t, copy, note) = ans("1000 iqd usd", true).unwrap();
        assert_eq!(t, "1,000 IQD = 0.763 USD");
        assert_eq!(copy, "0.763");
        assert!(!note.contains("EUR"));
        let (t, _, _) = ans("1 kwd", true).unwrap();
        assert_eq!(t, "1 KWD = 3.27 USD");
        let (t, _, _) = ans("100 تومان", true).unwrap();
        assert_eq!(t, "100 toman = 0.0238 USD");
    }

    #[test]
    fn fetched_rates_show_their_date() {
        let body = api_json(1_793_577_600, &[("IQD", 1320.0)]);
        let r = parse_api(body.as_bytes(), 1_793_600_000).unwrap();
        let (t, _, note) = answer("100 usd to iqd", false, &r).unwrap();
        assert_eq!(t, "100 دولار = 132,000 دينار عراقي");
        assert_eq!(note, "سعر تقريبي · 2026-11-02");
        let (_, _, note) = answer("100 eur", true, &r).unwrap();
        assert_eq!(note, "approximate rate · 2026-11-02");
    }

    #[test]
    fn money_formatting() {
        let f = |v: f64| format_money(v);
        assert_eq!(f(131_000.0).as_deref(), Some("131,000"));
        assert_eq!(f(1_234_567.89).as_deref(), Some("1,234,568"));
        assert_eq!(f(999.999).as_deref(), Some("1,000"));
        assert_eq!(f(86.0).as_deref(), Some("86"));
        assert_eq!(f(3.256).as_deref(), Some("3.26"));
        assert_eq!(f(0.763_358).as_deref(), Some("0.763"));
        assert_eq!(f(0.000_123_45).as_deref(), Some("0.000123"));
        assert_eq!(f(0.0), None);
        assert_eq!(f(-1.0), None);
        assert_eq!(f(f64::NAN), None);
        assert_eq!(f(f64::INFINITY), None);
        assert_eq!(f(1e16), None);
        assert_eq!(f(1e-15), None);
    }

    #[test]
    fn dates() {
        assert_eq!(date_text(0), "1970-01-01");
        assert_eq!(date_text(BUNDLED_UPDATED), "2026-10-01");
        assert_eq!(date_text(1_793_577_600), "2026-11-02");
        assert_eq!(date_text(951_782_400), "2000-02-29");
    }

    /// An API answer with the bundled rates, some of them overridden.
    fn api_json(updated: i64, over: &[(&str, f64)]) -> String {
        let mut rates = serde_json::Map::new();
        rates.insert("USD".into(), 1.into());
        for cur in CURRENCIES.iter().filter(|c| c.code != TOMAN_CODE) {
            rates.insert(cur.code.into(), cur.bundled.into());
        }
        rates.insert("XAU".into(), 0.0004.into());
        for (k, v) in over {
            rates.insert((*k).into(), (*v).into());
        }
        serde_json::json!({"result": "success", "base_code": "USD", "time_last_update_unix": updated, "rates": rates}).to_string()
    }

    const NOW: i64 = 1_793_600_000;

    #[test]
    fn good_api_answer() {
        let r = parse_api(api_json(NOW - 3600, &[("IQD", 1309.5), ("EUR", 0.9)]).as_bytes(), NOW).unwrap();
        assert_eq!(r.rate("IQD"), Some(1309.5));
        assert_eq!(r.rate("EUR"), Some(0.9));
        assert_eq!(r.rate("TOMAN"), Some(4200.0));
        assert_eq!((r.updated, r.fetched), (NOW - 3600, NOW));
        assert!(!r.is_bundled());
        // One broken minor rate keeps its bundled value.
        let r = parse_api(api_json(NOW, &[("SYP", -3.0)]).as_bytes(), NOW).unwrap();
        assert_eq!(r.rate("SYP"), Some(13_000.0));
    }

    #[test]
    fn bad_api_answers_are_refused() {
        let bad: Vec<String> = vec![
            String::new(),
            "{".into(),
            "null".into(),
            "[]".into(),
            "\"x\"".into(),
            r#"{"result":"error","error-type":"invalid-key"}"#.into(),
            r#"{"result":"success"}"#.into(),
            r#"{"result":"success","time_last_update_unix":1793577600}"#.into(),
            r#"{"result":"success","time_last_update_unix":1793577600,"rates":[1,2]}"#.into(),
            api_json(NOW, &[("USD", 2.0)]),
            api_json(NOW, &[("IQD", 0.0)]),
            api_json(NOW, &[("IQD", 1e9)]),
            api_json(NOW, &[("IQD", 13.1)]),
            api_json(NOW, &[("EUR", -0.86)]),
            api_json(1_000, &[]),
            api_json(NOW + 30 * 86_400, &[]),
            api_json(NOW, &[]).replace("\"USD\",", "\"EUR\","),
            api_json(NOW, &[]).replace("\"IQD\":1310.0", "\"IQD\":\"1310\""),
            api_json(NOW, &[]).replace("\"time_last_update_unix\":1793600000", "\"time_last_update_unix\":\"soon\""),
        ];
        for b in &bad {
            assert!(parse_api(b.as_bytes(), NOW).is_err(), "{b}");
        }
        // Too many currencies broken.
        let broken: Vec<(&str, f64)> = ["SAR", "AED", "KWD", "QAR", "BHD", "OMR"].iter().map(|c| (*c, f64::MAX)).collect();
        assert!(parse_api(api_json(NOW, &broken).as_bytes(), NOW).is_err());
        // Oversized.
        let big = format!("{}{}", api_json(NOW, &[]), " ".repeat(MAX_JSON));
        assert!(parse_api(big.as_bytes(), NOW).is_err());
        // Random bytes never panic.
        let mut x = 99u64;
        for _ in 0..2000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let n = (x >> 56) as usize;
            let bytes: Vec<u8> = (0..n).map(|k| (x >> (k % 57)) as u8).collect();
            let _ = parse_api(&bytes, NOW);
        }
    }

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(name: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("bahth-currency-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cache_round_trip_and_damage() {
        let dir = TempDir::new("cache");
        let path = dir.0.join("sub").join("rates.json");
        assert_eq!(load_cache(&path, NOW), None);
        let r = parse_api(api_json(NOW - 100, &[("IQD", 1311.0)]).as_bytes(), NOW).unwrap();
        save_cache(&path, &r).unwrap();
        assert_eq!(load_cache(&path, NOW), Some(r.clone()));
        for junk in ["", "{}", "garbage", r#"{"version":2,"updated":1,"fetched":1,"rates":{}}"#] {
            std::fs::write(&path, junk).unwrap();
            assert_eq!(load_cache(&path, NOW), None, "{junk}");
        }
        std::fs::write(&path, r.to_json().replace("1311.0", "-1311.0")).unwrap();
        assert_eq!(load_cache(&path, NOW), None);
        std::fs::write(&path, " ".repeat(MAX_JSON + 1)).unwrap();
        assert_eq!(load_cache(&path, NOW), None);
    }

    #[test]
    fn refresh_schedule() {
        assert!(is_due(0, NOW));
        assert!(!is_due(NOW - 3600, NOW));
        assert!(is_due(NOW - REFRESH_SECS, NOW));
        assert!(is_due(NOW + 3600, NOW)); // the clock went back
        let dir = TempDir::new("refresh");
        let path = dir.0.join("rates.json");
        let mut got: Vec<Rates> = Vec::new();
        // No cache: downloads, saves, installs.
        let body = api_json(NOW - 60, &[("IQD", 1312.0)]);
        let ran = refresh_with(&path, NOW, || Ok(body.clone().into_bytes()), &mut |r| got.push(r)).unwrap();
        assert!(ran);
        assert_eq!(got.len(), 1);
        assert_eq!(load_cache(&path, NOW).unwrap().rate("IQD"), Some(1312.0));
        // Fresh cache: installed, no download.
        got.clear();
        let ran = refresh_with(&path, NOW + 3600, || panic!("must not download"), &mut |r| got.push(r)).unwrap();
        assert!(!ran);
        assert_eq!(got[0].rate("IQD"), Some(1312.0));
        // Stale cache: still installed first; a failed download keeps it.
        got.clear();
        let later = NOW + REFRESH_SECS + 1;
        assert!(refresh_with(&path, later, || Err("offline".into()), &mut |r| got.push(r)).is_err());
        assert_eq!(got.len(), 1);
        assert_eq!(load_cache(&path, later).unwrap().rate("IQD"), Some(1312.0));
        // Bad data is not saved over the good cache.
        got.clear();
        assert!(refresh_with(&path, later, || Ok(b"{\"result\":\"success\"}".to_vec()), &mut |r| got.push(r)).is_err());
        assert_eq!(load_cache(&path, later).unwrap().rate("IQD"), Some(1312.0));
        // A good download replaces it.
        let body = api_json(later - 60, &[("IQD", 1305.0)]);
        assert!(refresh_with(&path, later, || Ok(body.into_bytes()), &mut |r| got.push(r)).unwrap());
        assert_eq!(got.last().unwrap().rate("IQD"), Some(1305.0));
        assert_eq!(load_cache(&path, later).unwrap().fetched, later);
    }

    #[test]
    fn current_starts_with_the_bundled_table() {
        // Nothing in tests installs rates older than the bundled ones.
        let c = current();
        assert!(c.updated >= BUNDLED_UPDATED);
    }

    #[test]
    fn random_input_never_panics() {
        let pieces = ["1", "٢", "100", "$", "usd", "دولار", "بال", "دينار", " ", "to", "عراقي", "كويتي", ".", ",", "€", "الى", "ـ", "😀", "د.ع", "-"];
        let mut x = 7u64;
        for _ in 0..20_000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let mut s = String::new();
            for k in 0..((x >> 60) + 1) {
                s.push_str(pieces[((x >> (k * 4 + 2)) % pieces.len() as u64) as usize]);
            }
            let _ = ans(&s, false);
            let _ = ans(&s, true);
        }
    }
}
