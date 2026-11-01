//! Hijri (Islamic) dates for the instant answers: today's Hijri date and
//! conversion both ways.
//!
//! 1365–1500 AH (5 Dec 1945 – 16 Nov 2077) follow the Umm al-Qura calendar
//! of Saudi Arabia, from the official month table (one bit per month, set
//! for a 30-day month). Outside that range the tabular Islamic civil
//! calendar is used and the answer is marked approximate (±1 day). Before
//! 1365 the official table has irregular 28- and 31-day months, which the
//! bit encoding cannot hold, so those years use the approximation too.

use std::sync::OnceLock;

use super::calc;
use super::units::norm;

const UQ_FIRST_YEAR: i32 = 1365;
/// Julian day number of 1 Muharram 1365 (5 December 1945).
const UQ_START_JDN: i64 = 2_431_795;

/// Month lengths of 1365–1500 AH: bit `m` (0 = Muharram) set = 30 days.
#[rustfmt::skip]
static UQ_MONTHS: [u16; 136] = [
    0xD55, 0x555, 0x555, 0xD55, 0x6D5, 0x555, 0xEA5, 0xD2A, 0xAAA, 0xCD5, 0x655, 0x572,
    0xDA9, 0x555, 0xAAA, 0x555, 0x52D, 0xA6D, 0x55A, 0x555, 0x74D, 0xD53, 0xD54, 0x556,
    0xD55, 0x2D5, 0xD55, 0xD54, 0xD45, 0x655, 0x52D, 0xA5D, 0x55A, 0xAD5, 0x6AA, 0xD4B,
    0x52A, 0xA57, 0x4AE, 0x976, 0x56C, 0xB55, 0xAAA, 0xA55, 0x4AD, 0x95D, 0x2DA, 0x5D9,
    0xDB2, 0xBA4, 0xB4A, 0xA55, 0x2B5, 0x575, 0xB6A, 0xBD2, 0xBC4, 0xB89, 0xA95, 0x52D,
    0x5AD, 0xB6A, 0x6D4, 0xDC9, 0xD92, 0xAA6, 0x956, 0x2AE, 0x56D, 0x36A, 0xB55, 0xAAA,
    0x94D, 0x49D, 0x95D, 0x2BA, 0x5B5, 0x5AA, 0xD55, 0xA9A, 0x92E, 0x26E, 0x55D, 0xADA,
    0x6D4, 0x6A5, 0x54B, 0xA97, 0x54E, 0xAAE, 0x5AC, 0xBA9, 0xD92, 0xB25, 0x64B, 0xCAB,
    0x55A, 0xB55, 0x6D2, 0xEA5, 0xE4A, 0xA95, 0x52D, 0xAAD, 0x36C, 0x759, 0x6D2, 0x695,
    0x52D, 0xA5B, 0x4BA, 0x9BA, 0x3B4, 0xB69, 0xB52, 0xAA6, 0x4B6, 0x96D, 0x2EC, 0x6D9,
    0xEB2, 0xD54, 0xD2A, 0xA56, 0x4AE, 0x96D, 0xD6A, 0xB54, 0xB29, 0xA93, 0x52B, 0xA57,
    0x536, 0xAB5, 0x6AA, 0xE93,
];

const UQ_LAST_YEAR: i32 = UQ_FIRST_YEAR + UQ_MONTHS.len() as i32 - 1;

fn uq_month_len(y: i32, m: u32) -> u32 {
    let bits = UQ_MONTHS[(y - UQ_FIRST_YEAR) as usize];
    29 + u32::from((bits >> (m - 1)) & 1)
}

fn uq_year_len(y: i32) -> i64 {
    348 + i64::from(UQ_MONTHS[(y - UQ_FIRST_YEAR) as usize].count_ones())
}

/// JDN one past the end of the table (1 Muharram 1501).
fn uq_end_jdn() -> i64 {
    UQ_START_JDN + (UQ_FIRST_YEAR..=UQ_LAST_YEAR).map(uq_year_len).sum::<i64>()
}

// --- Gregorian ---------------------------------------------------------------

pub fn is_leap_gregorian(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn gregorian_month_len(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_gregorian(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Julian day number of a Gregorian date, `None` if it is not a real date
/// (or outside years 1–9999).
pub fn gregorian_to_jdn(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d == 0 || d > gregorian_month_len(y, m) {
        return None;
    }
    let (y, m, d) = (i64::from(y), i64::from(m), i64::from(d));
    let a = (14 - m) / 12;
    let yy = y + 4800 - a;
    let mm = m + 12 * a - 3;
    Some(d + (153 * mm + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32045)
}

pub fn jdn_to_gregorian(j: i64) -> (i32, u32, u32) {
    let a = j + 32044;
    let b = (4 * a + 3) / 146_097;
    let c = a - 146_097 * b / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - 1461 * d / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    (year as i32, month as u32, day as u32)
}

/// 0 = Sunday … 6 = Saturday.
pub fn weekday(j: i64) -> usize {
    (j + 1).rem_euclid(7) as usize
}

// --- Hijri -------------------------------------------------------------------

/// Tabular Islamic civil calendar (epoch 16 July 622 Julian).
const CIVIL_EPOCH: i64 = 1_948_440;

fn civil_leap(y: i32) -> bool {
    (14 + 11 * i64::from(y)).rem_euclid(30) < 11
}

fn civil_month_len(y: i32, m: u32) -> u32 {
    if m % 2 == 1 || (m == 12 && civil_leap(y)) { 30 } else { 29 }
}

fn civil_to_jdn(y: i32, m: u32, d: u32) -> i64 {
    let (y, m, d) = (i64::from(y), i64::from(m), i64::from(d));
    d + (59 * (m - 1) + 1) / 2 + (y - 1) * 354 + (3 + 11 * y).div_euclid(30) + CIVIL_EPOCH - 1
}

fn jdn_to_civil(j: i64) -> (i32, u32, u32) {
    let y = (30 * (j - CIVIL_EPOCH) + 10646).div_euclid(10631);
    let y = y as i32;
    let mut m = 1;
    while m < 12 && civil_to_jdn(y, m + 1, 1) <= j {
        m += 1;
    }
    let d = (j - civil_to_jdn(y, m, 1) + 1) as u32;
    (y, m, d)
}

/// Days added to the civil calendar outside the table so that it joins the
/// table without a gap or overlap (before 1365 or after 1500).
fn civil_shift(before: bool) -> i64 {
    if before {
        UQ_START_JDN - civil_to_jdn(UQ_FIRST_YEAR, 1, 1)
    } else {
        uq_end_jdn() - civil_to_jdn(UQ_LAST_YEAR + 1, 1, 1)
    }
}

/// Length of a Hijri month (Umm al-Qura where known).
pub fn hijri_month_len(y: i32, m: u32) -> u32 {
    if (UQ_FIRST_YEAR..=UQ_LAST_YEAR).contains(&y) { uq_month_len(y, m) } else { civil_month_len(y, m) }
}

/// Julian day number of a Hijri date and whether it is exact (Umm al-Qura)
/// rather than approximate. `None` for an impossible date or a year outside 1–9999.
pub fn hijri_to_jdn(y: i32, m: u32, d: u32) -> Option<(i64, bool)> {
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d == 0 || d > hijri_month_len(y, m) {
        return None;
    }
    if (UQ_FIRST_YEAR..=UQ_LAST_YEAR).contains(&y) {
        let mut j = UQ_START_JDN;
        for yy in UQ_FIRST_YEAR..y {
            j += uq_year_len(yy);
        }
        for mm in 1..m {
            j += i64::from(uq_month_len(y, mm));
        }
        return Some((j + i64::from(d) - 1, true));
    }
    Some((civil_to_jdn(y, m, d) + civil_shift(y < UQ_FIRST_YEAR), false))
}

/// The Hijri date of a Julian day number and whether it is exact.
pub fn jdn_to_hijri(j: i64) -> (i32, u32, u32, bool) {
    if (UQ_START_JDN..uq_end_jdn()).contains(&j) {
        let mut start = UQ_START_JDN;
        let mut y = UQ_FIRST_YEAR;
        while start + uq_year_len(y) <= j {
            start += uq_year_len(y);
            y += 1;
        }
        let mut m = 1;
        while start + i64::from(uq_month_len(y, m)) <= j {
            start += i64::from(uq_month_len(y, m));
            m += 1;
        }
        return (y, m, (j - start + 1) as u32, true);
    }
    let before = j < UQ_START_JDN;
    let (y, m, d) = jdn_to_civil(j - civil_shift(before));
    (y, m, d, false)
}

// --- Names -------------------------------------------------------------------

const HIJRI_AR: [&str; 12] =
    ["محرم", "صفر", "ربيع الأول", "ربيع الآخر", "جمادى الأولى", "جمادى الآخرة", "رجب", "شعبان", "رمضان", "شوال", "ذو القعدة", "ذو الحجة"];
const HIJRI_EN: [&str; 12] = [
    "Muharram", "Safar", "Rabi al-Awwal", "Rabi al-Thani", "Jumada al-Ula", "Jumada al-Akhirah", "Rajab", "Shaban", "Ramadan",
    "Shawwal", "Dhu al-Qadah", "Dhu al-Hijjah",
];
const GREG_AR: [&str; 12] = ["يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو", "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر"];
const GREG_EN: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS_AR: [&str; 7] = ["الأحد", "الاثنين", "الثلاثاء", "الأربعاء", "الخميس", "الجمعة", "السبت"];
const DAYS_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Names a user may type for each Hijri month (normalised before matching).
const HIJRI_NAMES: [&[&str]; 12] = [
    &["محرم", "المحرم", "muharram", "muharam", "moharram", "muharrem"],
    &["صفر", "safar", "safer"],
    &[
        "ربيع الأول", "ربيع أول", "ربيع 1", "ربيع الاول", "rabi al awwal", "rabi ul awwal", "rabiul awwal", "rabi al awal",
        "rabi ul awal", "rabia al awwal", "rabi i", "rabi 1", "rabi al-awwal", "rabi1",
    ],
    &[
        "ربيع الآخر", "ربيع الثاني", "ربيع ثاني", "ربيع آخر", "ربيع 2", "rabi al thani", "rabi ul thani", "rabiul thani",
        "rabi al akhir", "rabi al akhar", "rabi al sani", "rabi us sani", "rabi ii", "rabi 2", "rabi2",
    ],
    &[
        "جمادى الأولى", "جمادى الأول", "جمادى أولى", "جمادى 1", "جمادى الاولي", "jumada al ula", "jumada al oula",
        "jumada al awwal", "jumada al ola", "jumadal ula", "jumada ul ula", "jumada ul awwal", "jumada i", "jumada 1",
        "jumada1", "jamadi ul awwal",
    ],
    &[
        "جمادى الآخرة", "جمادى الآخر", "جمادى الثانية", "جمادى الثاني", "جمادى آخرة", "جمادى 2", "jumada al akhirah",
        "jumada al akhira", "jumada al akhir", "jumada al thani", "jumada al thaniyah", "jumada ul akhir", "jumada ul thani",
        "jumada ii", "jumada 2", "jumada2", "jamadi us sani",
    ],
    &["رجب", "rajab", "rajeb"],
    &["شعبان", "shaban", "shaaban", "shabaan", "sha aban"],
    &["رمضان", "ramadan", "ramadhan", "ramazan", "ramzan"],
    &["شوال", "shawwal", "shawal", "shawwaal"],
    &[
        "ذو القعدة", "ذي القعدة", "ذوالقعدة", "ذا القعدة", "dhu al qadah", "dhu al qidah", "dhul qadah", "dhul qidah",
        "dhu al qada", "dhu al qadah", "zul qadah", "zul qida", "dhu l qada", "thul qadah", "dhu al qaadah",
    ],
    &[
        "ذو الحجة", "ذي الحجة", "ذوالحجة", "ذا الحجة", "dhu al hijjah", "dhu al hijja", "dhul hijjah", "dhul hijja",
        "zul hijjah", "zil hijjah", "dhu l hijja", "thul hijjah",
    ],
];

const GREG_NAMES: [&[&str]; 12] = [
    &["يناير", "كانون الثاني", "جانفي", "january", "jan"],
    &["فبراير", "شباط", "فيفري", "february", "feb"],
    &["مارس", "آذار", "march", "mar"],
    &["أبريل", "إبريل", "نيسان", "افريل", "april", "apr"],
    &["مايو", "أيار", "ماي", "may"],
    &["يونيو", "يونيه", "حزيران", "جوان", "june", "jun"],
    &["يوليو", "يوليه", "تموز", "جويلية", "july", "jul"],
    &["أغسطس", "آب", "أوت", "august", "aug"],
    &["سبتمبر", "أيلول", "september", "sept", "sep"],
    &["أكتوبر", "تشرين الأول", "october", "oct"],
    &["نوفمبر", "تشرين الثاني", "november", "nov"],
    &["ديسمبر", "كانون الأول", "december", "dec"],
];

/// Normalised text for date matching: `norm`, without apostrophes and with
/// hyphens, commas and dots between words turned into spaces.
fn date_norm(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !matches!(c, '\'' | '’' | '‘' | 'ʿ' | 'ʾ' | '`' | '?' | '؟'))
        .map(|c| if matches!(c, ',' | '،') { ' ' } else { c })
        .collect();
    let n = norm(&cleaned);
    // Split digits from glued letters: "1447هـ" → "1447 ه", "20مارس" → "20 مارس".
    let mut out = String::with_capacity(n.len() + 8);
    let mut prev: Option<char> = None;
    for c in n.chars() {
        if let Some(p) = prev
            && ((p.is_ascii_digit() && c.is_alphabetic()) || (p.is_alphabetic() && c.is_ascii_digit()))
        {
            out.push(' ');
        }
        out.push(if c == '-' && prev.is_some_and(|p| p.is_alphabetic()) { ' ' } else { c });
        prev = Some(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cal {
    Hijri,
    Gregorian,
}

/// What a date query asks for.
#[derive(Debug, PartialEq)]
enum Ask {
    TodayHijri,
    TodayBoth,
    /// Convert a date given in this calendar.
    Convert(Cal, i32, u32, u32),
}

const TODAY_HIJRI: &[&str] = &[
    "هجري", "هجرية", "الهجري", "بالهجري", "التاريخ الهجري", "تاريخ هجري", "تاريخ اليوم هجري", "تاريخ اليوم بالهجري",
    "اليوم هجري", "اليوم بالهجري", "هجري اليوم", "التاريخ بالهجري", "كم التاريخ هجري", "كم التاريخ بالهجري",
    "hijri", "hijri date", "islamic date", "today hijri", "hijri today", "date hijri", "ummalqura", "umm al qura",
];
const TODAY_BOTH: &[&str] = &[
    "تاريخ", "التاريخ", "اليوم", "تاريخ اليوم", "ما التاريخ", "كم التاريخ", "التاريخ اليوم", "ميلادي", "التاريخ الميلادي",
    "تاريخ ميلادي", "today", "date", "todays date", "today date", "gregorian", "gregorian date", "what is the date",
    "what date is it",
];
const HIJRI_MARKS: &[&str] = &["هجري", "هجرية", "الهجري", "ه", "هـ", "ah", "hijri", "h"];
const GREG_MARKS: &[&str] = &["ميلادي", "ميلادية", "الميلادي", "م", "ad", "ce", "gregorian", "g"];
const TARGET_HIJRI: &[&str] = &["بالهجري", "للهجري"];
const TARGET_GREG: &[&str] = &["بالميلادي", "للميلادي"];
const CONVERT_WORDS: &[&str] = &["to", "in", "الى", "إلى", "convert", "حول", "تحويل", "كم يوافق", "يوافق", "يساوي", "="];

/// The word lists above and the month names, normalised once.
struct Lists {
    today_hijri: Vec<String>,
    today_both: Vec<String>,
    hijri_marks: Vec<String>,
    greg_marks: Vec<String>,
    target_hijri: Vec<String>,
    target_greg: Vec<String>,
    convert: Vec<String>,
    /// (words of the name, month, calendar)
    months: Vec<(Vec<String>, u32, Cal)>,
}

fn lists() -> &'static Lists {
    static L: OnceLock<Lists> = OnceLock::new();
    L.get_or_init(|| {
        let n = |l: &[&str]| l.iter().map(|w| date_norm(w)).collect::<Vec<_>>();
        let mut months = Vec::new();
        for (cal, table) in [(Cal::Hijri, &HIJRI_NAMES), (Cal::Gregorian, &GREG_NAMES)] {
            for (mi, names) in table.iter().enumerate() {
                for name in names.iter() {
                    let parts = date_norm(name).split(' ').map(str::to_string).collect();
                    months.push((parts, mi as u32 + 1, cal));
                }
            }
        }
        Lists {
            today_hijri: n(TODAY_HIJRI),
            today_both: n(TODAY_BOTH),
            hijri_marks: n(HIJRI_MARKS),
            greg_marks: n(GREG_MARKS),
            target_hijri: n(TARGET_HIJRI),
            target_greg: n(TARGET_GREG),
            convert: n(CONVERT_WORDS),
            months,
        }
    })
}

fn in_list(word: &str, list: &[String]) -> bool {
    list.iter().any(|w| w == word)
}

/// The month named at the start of `words[i..]`: (month, calendar, words used).
fn month_at(words: &[&str], i: usize) -> Option<(u32, Cal, usize)> {
    let mut best: Option<(u32, Cal, usize)> = None;
    for (parts, m, cal) in &lists().months {
        let k = parts.len();
        if words.len() >= i + k && words[i..i + k].iter().zip(parts).all(|(a, b)| a == b) && best.is_none_or(|b| k > b.2) {
            best = Some((*m, *cal, k));
        }
    }
    best
}

fn parse_ask(query: &str, today_hijri_year: i32) -> Option<Ask> {
    if query.chars().count() > 80 {
        return None;
    }
    let q = date_norm(query);
    if q.is_empty() {
        return None;
    }
    let l = lists();
    if in_list(&q, &l.today_hijri) {
        return Some(Ask::TodayHijri);
    }
    if in_list(&q, &l.today_both) {
        return Some(Ask::TodayBoth);
    }
    // Everything else needs a number, which keeps ordinary searches fast.
    if !q.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    let words: Vec<&str> = q.split(' ').collect();
    let mut nums: Vec<&str> = Vec::new();
    let mut month: Option<(u32, Cal)> = None;
    let mut source: Option<Cal> = None;
    let mut target: Option<Cal> = None;
    let mut after_connector = false;
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if let Some((m, cal, used)) = month_at(&words, i) {
            if month.is_some() {
                return None;
            }
            month = Some((m, cal));
            i += used;
            continue;
        }
        if !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'/' | b'-' | b'.')) {
            nums.push(w);
        } else if in_list(w, &l.convert) {
            after_connector = true;
        } else if in_list(w, &l.target_hijri) {
            target = Some(Cal::Hijri);
        } else if in_list(w, &l.target_greg) {
            target = Some(Cal::Gregorian);
        } else if in_list(w, &l.hijri_marks) || in_list(w, &l.greg_marks) {
            let cal = if in_list(w, &l.hijri_marks) { Cal::Hijri } else { Cal::Gregorian };
            match source {
                // "… هجري ميلادي": the second calendar is the one wanted.
                Some(s) if s != cal => target = Some(cal),
                _ if after_connector => target = Some(cal),
                None => source = Some(cal),
                Some(_) => {}
            }
        } else {
            return None;
        }
        i += 1;
    }
    let (y, m, d, inferred) = if let Some((m, cal)) = month {
        let n: Vec<u32> = nums.iter().map(|s| s.parse::<u32>().ok()).collect::<Option<Vec<_>>>()?;
        let (d, y) = match n[..] {
            [d] if cal == Cal::Hijri && (1..=30).contains(&d) => (d, today_hijri_year),
            [d, y] | [y, d] if (1..=31).contains(&d) && y > 31 => (d, y as i32),
            _ => return None,
        };
        if source.is_some_and(|s| s != cal) {
            return None;
        }
        (y, m, d, Some(cal))
    } else {
        if nums.len() != 1 {
            return None;
        }
        let parts: Vec<&str> = nums[0].split(['/', '-', '.']).collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || p.len() > 4) {
            return None;
        }
        let sep_count = nums[0].chars().filter(|c| matches!(c, '/' | '-' | '.')).count();
        let first_sep = nums[0].chars().find(|c| matches!(c, '/' | '-' | '.'))?;
        if sep_count != 2 || nums[0].chars().filter(|&c| c == first_sep).count() != 2 {
            return None;
        }
        let n: Vec<u32> = parts.iter().map(|p| p.parse::<u32>().ok()).collect::<Option<Vec<_>>>()?;
        if parts[0].len() >= 3 {
            (n[0] as i32, n[1], n[2], None)
        } else if parts[2].len() >= 3 {
            // Day first, as in Arabic usage, unless the middle can only be a day.
            if n[1] > 12 && n[0] <= 12 { (n[2] as i32, n[0], n[1], None) } else { (n[2] as i32, n[1], n[0], None) }
        } else {
            return None;
        }
    };
    let from = inferred.or(source).unwrap_or(if (1300..=1500).contains(&y) { Cal::Hijri } else { Cal::Gregorian });
    if target == Some(from) {
        return None;
    }
    Some(Ask::Convert(from, y, m, d))
}

fn fmt_hijri(y: i32, m: u32, d: u32, english: bool) -> String {
    if english {
        format!("{d} {} {y} AH", HIJRI_EN[(m - 1) as usize])
    } else {
        format!("{d} {} {y} هـ", HIJRI_AR[(m - 1) as usize])
    }
}

fn fmt_gregorian(j: i64, english: bool) -> String {
    let (y, m, d) = jdn_to_gregorian(j);
    let wd = weekday(j);
    if english {
        format!("{}, {d} {} {y}", DAYS_EN[wd], GREG_EN[(m - 1) as usize])
    } else {
        format!("{} {d} {} {y} م", DAYS_AR[wd], GREG_AR[(m - 1) as usize])
    }
}

fn label(exact: bool, english: bool) -> &'static str {
    match (exact, english) {
        (true, false) => "تقويم أم القرى",
        (true, true) => "Umm al-Qura calendar",
        (false, false) => "تقريبي (±1 يوم)",
        (false, true) => "approximate (±1 day)",
    }
}

/// A date answer: (title, text to copy, note for the subtitle).
pub fn answer(query: &str, today: (i32, u32, u32), english: bool) -> Option<(String, String, String)> {
    let digits = calc::detect_digits(query);
    let today_jdn = gregorian_to_jdn(today.0, today.1, today.2);
    let today_hijri_year = today_jdn.map(|j| jdn_to_hijri(j).0).unwrap_or(1448);
    let ask = parse_ask(query, today_hijri_year)?;
    let loc = |s: String| calc::localize(&s, digits);
    let (title, copy, exact) = match ask {
        Ask::TodayHijri => {
            let j = today_jdn?;
            let (y, m, d, exact) = jdn_to_hijri(j);
            let h = fmt_hijri(y, m, d, english);
            (h.clone(), h, exact)
        }
        Ask::TodayBoth => {
            let j = today_jdn?;
            let (y, m, d, exact) = jdn_to_hijri(j);
            let both = format!("{} · {}", fmt_gregorian(j, english), fmt_hijri(y, m, d, english));
            (both.clone(), both, exact)
        }
        Ask::Convert(Cal::Hijri, y, m, d) => {
            let (j, exact) = hijri_to_jdn(y, m, d)?;
            let g = fmt_gregorian(j, english);
            (format!("{} = {g}", fmt_hijri(y, m, d, english)), g, exact)
        }
        Ask::Convert(Cal::Gregorian, y, m, d) => {
            let j = gregorian_to_jdn(y, m, d)?;
            let (hy, hm, hd, exact) = jdn_to_hijri(j);
            if hy < 1 {
                return None;
            }
            let h = fmt_hijri(hy, hm, hd, english);
            (format!("{} = {h}", fmt_gregorian(j, english)), h, exact)
        }
    };
    Some((loc(title), loc(copy), label(exact, english).to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h2g(y: i32, m: u32, d: u32) -> (i32, u32, u32) {
        jdn_to_gregorian(hijri_to_jdn(y, m, d).unwrap().0)
    }

    fn g2h(y: i32, m: u32, d: u32) -> (i32, u32, u32, bool) {
        jdn_to_hijri(gregorian_to_jdn(y, m, d).unwrap())
    }

    #[test]
    fn gregorian_round_trip() {
        assert_eq!(gregorian_to_jdn(2000, 1, 1), Some(2_451_545));
        assert_eq!(jdn_to_gregorian(2_451_545), (2000, 1, 1));
        assert_eq!(weekday(2_451_545), 6); // Saturday
        assert_eq!(gregorian_to_jdn(2023, 2, 29), None);
        assert!(gregorian_to_jdn(2024, 2, 29).is_some());
        assert_eq!(gregorian_to_jdn(1900, 2, 29), None);
        assert_eq!(gregorian_to_jdn(2026, 13, 1), None);
        assert_eq!(gregorian_to_jdn(0, 1, 1), None);
        let start = gregorian_to_jdn(1900, 1, 1).unwrap();
        for j in (start..start + 80_000).step_by(37) {
            let (y, m, d) = jdn_to_gregorian(j);
            assert_eq!(gregorian_to_jdn(y, m, d), Some(j));
        }
    }

    /// Published Umm al-Qura dates (Saudi announcements and ummulqura.org.sa).
    #[test]
    fn umm_al_qura_known_dates() {
        assert_eq!(h2g(1447, 9, 1), (2026, 2, 18)); // 1 Ramadan 1447
        assert_eq!(h2g(1448, 1, 1), (2026, 6, 16)); // 1 Muharram 1448
        assert_eq!(h2g(1447, 10, 1), (2026, 3, 20)); // Eid al-Fitr 1447
        assert_eq!(h2g(1447, 1, 1), (2025, 6, 26)); // 1 Muharram 1447
        assert_eq!(h2g(1446, 9, 1), (2025, 3, 1)); // 1 Ramadan 1446
        assert_eq!(h2g(1446, 10, 1), (2025, 3, 30)); // Eid al-Fitr 1446
        assert_eq!(h2g(1446, 12, 10), (2025, 6, 6)); // Eid al-Adha 1446
        assert_eq!(h2g(1445, 9, 1), (2024, 3, 11)); // 1 Ramadan 1445
        assert_eq!(h2g(1445, 10, 1), (2024, 4, 10)); // Eid al-Fitr 1445
        assert_eq!(h2g(1440, 10, 1), (2019, 6, 4)); // Eid al-Fitr 1440
        assert_eq!(h2g(1430, 1, 1), (2008, 12, 29)); // 1 Muharram 1430
        assert_eq!(h2g(1420, 9, 1), (1999, 12, 9)); // 1 Ramadan 1420
        assert_eq!(g2h(2026, 11, 1), (1448, 5, 21, true));
        assert_eq!(g2h(1945, 12, 5), (1365, 1, 1, true));
        assert_eq!(g2h(2077, 11, 16), (1500, 12, 30, true));
        assert!(!g2h(2077, 11, 17).3);
        assert!(!g2h(1945, 12, 4).3);
    }

    #[test]
    fn table_round_trip_and_lengths() {
        assert_eq!(UQ_LAST_YEAR, 1500);
        assert_eq!(jdn_to_gregorian(uq_end_jdn()), (2077, 11, 17));
        for y in UQ_FIRST_YEAR..=UQ_LAST_YEAR {
            let len = uq_year_len(y);
            assert!((353..=356).contains(&len), "{y}: {len}");
            for m in 1..=12 {
                for d in [1, hijri_month_len(y, m)] {
                    let (j, exact) = hijri_to_jdn(y, m, d).unwrap();
                    assert!(exact);
                    assert_eq!(jdn_to_hijri(j), (y, m, d, true));
                }
            }
        }
        // Tabular years round-trip too.
        for y in [1, 2, 100, 1000, 1300, 1364, 1501, 1600, 9999] {
            for m in 1..=12 {
                for d in [1, hijri_month_len(y, m)] {
                    let (j, exact) = hijri_to_jdn(y, m, d).unwrap();
                    assert!(!exact);
                    assert_eq!(jdn_to_hijri(j), (y, m, d, false), "{y}-{m}-{d}");
                }
            }
        }
        // The civil approximation stays within a couple of days of Umm al-Qura.
        for y in UQ_FIRST_YEAR..=UQ_LAST_YEAR {
            let a = hijri_to_jdn(y, 1, 1).unwrap().0;
            let b = civil_to_jdn(y, 1, 1);
            assert!((a - b).abs() <= 2, "{y}: {}", a - b);
        }
        assert_eq!(hijri_to_jdn(1447, 13, 1), None);
        assert_eq!(hijri_to_jdn(1447, 1, 31), None);
        assert_eq!(hijri_to_jdn(0, 1, 1), None);
    }

    /// Day after day, across both ends of the table, the Hijri date moves by
    /// exactly one day: no gap, no repeated date.
    #[test]
    fn continuous_at_table_edges() {
        for center in [UQ_START_JDN, uq_end_jdn()] {
            let mut prev = jdn_to_hijri(center - 400);
            for j in center - 399..center + 400 {
                let cur = jdn_to_hijri(j);
                let (py, pm, pd, _) = prev;
                let next = if pd < hijri_month_len(py, pm) {
                    (py, pm, pd + 1)
                } else if pm < 12 {
                    (py, pm + 1, 1)
                } else {
                    (py + 1, 1, 1)
                };
                assert_eq!((cur.0, cur.1, cur.2), next, "jdn {j}");
                assert_eq!(hijri_to_jdn(cur.0, cur.1, cur.2).map(|r| r.0), Some(j));
                prev = cur;
            }
        }
    }

    const TODAY: (i32, u32, u32) = (2026, 11, 1);

    fn ans(q: &str, en: bool) -> Option<(String, String, String)> {
        answer(q, TODAY, en)
    }

    #[test]
    fn today_queries() {
        let (t, c, note) = ans("هجري", false).unwrap();
        assert_eq!(t, "21 جمادى الأولى 1448 هـ");
        assert_eq!(c, t);
        assert_eq!(note, "تقويم أم القرى");
        assert_eq!(ans("التاريخ الهجري", false).unwrap().0, "21 جمادى الأولى 1448 هـ");
        assert_eq!(ans("Hijri", true).unwrap().0, "21 Jumada al-Ula 1448 AH");
        assert_eq!(ans("hijri date", true).unwrap().0, "21 Jumada al-Ula 1448 AH");
        assert_eq!(ans("تاريخ", false).unwrap().0, "الأحد 1 نوفمبر 2026 م · 21 جمادى الأولى 1448 هـ");
        assert_eq!(ans("اليوم", false).unwrap().0, "الأحد 1 نوفمبر 2026 م · 21 جمادى الأولى 1448 هـ");
        assert_eq!(ans("today", true).unwrap().0, "Sunday, 1 November 2026 · 21 Jumada al-Ula 1448 AH");
        assert!(answer("هجري", (2026, 2, 30), false).is_none());
    }

    #[test]
    fn hijri_to_gregorian_queries() {
        assert_eq!(ans("1/9/1447 هجري", false).unwrap().0, "1 رمضان 1447 هـ = الأربعاء 18 فبراير 2026 م");
        assert_eq!(ans("1/9/1447 هجري", false).unwrap().1, "الأربعاء 18 فبراير 2026 م");
        assert_eq!(ans("1447/9/1 هـ", false).unwrap().0, "1 رمضان 1447 هـ = الأربعاء 18 فبراير 2026 م");
        assert_eq!(ans("1447/9/1هـ", false).unwrap().0, "1 رمضان 1447 هـ = الأربعاء 18 فبراير 2026 م");
        assert_eq!(ans("1 رمضان 1447", false).unwrap().0, "1 رمضان 1447 هـ = الأربعاء 18 فبراير 2026 م");
        assert_eq!(ans("١ رمضان ١٤٤٧", false).unwrap().0, "١ رمضان ١٤٤٧ هـ = الأربعاء ١٨ فبراير ٢٠٢٦ م");
        assert_eq!(ans("1 Ramadan 1447", true).unwrap().0, "1 Ramadan 1447 AH = Wednesday, 18 February 2026");
        assert_eq!(ans("1 muharram 1448 ah", true).unwrap().1, "Tuesday, 16 June 2026");
        assert_eq!(ans("10 ذو الحجة 1446", false).unwrap().1, "الجمعة 6 يونيو 2025 م");
        assert_eq!(ans("10 ذي الحجة 1446 هجري", false).unwrap().1, "الجمعة 6 يونيو 2025 م");
        assert_eq!(ans("1 jumada al-ula 1448", true).unwrap().1, "Monday, 12 October 2026");
        assert_eq!(ans("15 ربيع الأول 1448", false).unwrap().1, "الجمعة 28 أغسطس 2026 م");
        assert_eq!(ans("1 rabi' al-awwal 1448", true).unwrap().1, "Friday, 14 August 2026");
        assert_eq!(ans("1 dhul-hijjah 1447", true).unwrap().1, "Monday, 18 May 2026");
        // No year: the current Hijri year.
        assert_eq!(ans("1 رمضان", false).unwrap().0, "1 رمضان 1448 هـ = الاثنين 8 فبراير 2027 م");
        // Without a marker, a year of 1300–1500 is Hijri.
        assert_eq!(ans("1/9/1447", false).unwrap().1, "الأربعاء 18 فبراير 2026 م");
        // Approximate outside the table.
        let (_, _, note) = ans("1/1/1300 هـ", false).unwrap();
        assert_eq!(note, "تقريبي (±1 يوم)");
        assert_eq!(ans("30/2/1447 هجري", false), None); // Safar 1447 has 29 days
    }

    #[test]
    fn gregorian_to_hijri_queries() {
        assert_eq!(ans("2026/3/20", false).unwrap().0, "الجمعة 20 مارس 2026 م = 1 شوال 1447 هـ");
        assert_eq!(ans("20/3/2026 ميلادي", false).unwrap().1, "1 شوال 1447 هـ");
        assert_eq!(ans("2026-03-20", true).unwrap().1, "1 Shawwal 1447 AH");
        assert_eq!(ans("3/20/2026", true).unwrap().1, "1 Shawwal 1447 AH"); // US order
        assert_eq!(ans("20 مارس 2026", false).unwrap().1, "1 شوال 1447 هـ");
        assert_eq!(ans("20 march 2026", true).unwrap().1, "1 Shawwal 1447 AH");
        assert_eq!(ans("March 20, 2026", true).unwrap().1, "1 Shawwal 1447 AH");
        assert_eq!(ans("18 فبراير 2026 بالهجري", false).unwrap().1, "1 رمضان 1447 هـ");
        assert_eq!(ans("2026/2/18 to hijri", true).unwrap().1, "1 Ramadan 1447 AH");
        assert_eq!(ans("٢٠٢٦/٢/١٨", false).unwrap().1, "١ رمضان ١٤٤٧ هـ");
        assert_eq!(ans("16 يونيو 2026 م", false).unwrap().1, "1 محرم 1448 هـ");
        assert_eq!(ans("1 1 1900", false), None);
        assert_eq!(ans("1/1/1900", false).unwrap().2, "تقريبي (±1 يوم)");
        assert_eq!(ans("1/9/1447 هجري ميلادي", false).unwrap().1, "الأربعاء 18 فبراير 2026 م");
    }

    #[test]
    fn not_dates() {
        for q in [
            "", " ", "تقرير", "2026", "1447", "مارس", "مارس 2026", "20 مارس", "صفر", "رمضان", "تاريخ الفن", "اليوم الوطني",
            "hijri calendar pdf", "2026/13/45", "31/2/2026", "2026/3", "1/2/3", "12/12/12", "2026/3/20/1", "2026-3/20",
            "1 رمضان 1447 مارس", "2026/3/20 بالميلادي", "1/9/1447 بالهجري", "😀", "1 رمضان 😀", "date night",
            "20 march 2026 report", "99999/1/1", "1/1/99999", "h", "g", "م", "ه",
        ] {
            assert_eq!(ans(q, false), None, "{q:?}");
        }
    }

    #[test]
    fn random_input_never_panics() {
        let alphabet: Vec<char> = "0123456789/-. ٠١٢٣٤٥٦٧٨٩هـجريمرضانشوالمارسhijrimayto😀".chars().collect();
        let mut x = 0x9E3779B97F4A7C15u64;
        for _ in 0..5000 {
            let mut s = String::new();
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
            let len = (x >> 60) as usize;
            for _ in 0..len {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                s.push(alphabet[((x >> 33) % alphabet.len() as u64) as usize]);
            }
            let _ = answer(&s, TODAY, false);
            let _ = answer(&s, (9999, 12, 31), true);
            let _ = answer(&s, (1, 1, 1), true);
        }
    }
}
