//! Unit conversion for the instant answers: "5 كيلو بالباوند", "5 kg to lb",
//! "30 مئوي", "100 F to C", "1 TB in GB", "100 كم/س بالميل/س".

use std::sync::OnceLock;

use super::calc::{self, Digits};
use crate::i18n::Lang;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dim {
    Length,
    Mass,
    Temperature,
    Area,
    Volume,
    Data,
    Time,
    Speed,
}

/// One unit: `value × factor + offset` is the value in the base unit of its
/// dimension (metre, kilogram, kelvin, m², litre, byte, second, m/s).
struct Unit {
    id: &'static str,
    dim: Dim,
    factor: f64,
    offset: f64,
    ar: &'static str,
    en: &'static str,
    /// The unit shown when the query names none ("1 ميل" → km).
    default_to: &'static str,
    aliases: &'static [&'static str],
}

const fn u(
    id: &'static str,
    dim: Dim,
    factor: f64,
    ar: &'static str,
    en: &'static str,
    default_to: &'static str,
    aliases: &'static [&'static str],
) -> Unit {
    Unit { id, dim, factor, offset: 0.0, ar, en, default_to, aliases }
}

const DAY: f64 = 86_400.0;

static UNITS: &[Unit] = &[
    // Length (metre).
    u("mm", Dim::Length, 0.001, "ملم", "mm", "in", &["mm", "millimeter", "millimeters", "millimetre", "millimetres", "ملم", "مم", "مليمتر", "ملليمتر", "ميليمتر", "ملي متر", "مليمترات"]),
    u("cm", Dim::Length, 0.01, "سم", "cm", "in", &["cm", "centimeter", "centimeters", "centimetre", "centimetres", "سم", "سنتيمتر", "سنتي", "سانتي", "سانتيمتر", "سنتي متر", "سنتيمترات", "سانتيمترات"]),
    u("m", Dim::Length, 1.0, "متر", "m", "ft", &["m", "meter", "meters", "metre", "metres", "م", "متر", "مترا", "أمتار"]),
    u("km", Dim::Length, 1000.0, "كم", "km", "mi", &["km", "kms", "kilometer", "kilometers", "kilometre", "kilometres", "كم", "كلم", "كيلومتر", "كيلو متر", "كيلومترات", "كيلو مترات"]),
    u("in", Dim::Length, 0.0254, "إنش", "in", "cm", &["in", "inch", "inches", "انش", "إنش", "انشات", "إنشات", "بوصة", "بوصات", "″"]),
    u("ft", Dim::Length, 0.3048, "قدم", "ft", "m", &["ft", "foot", "feet", "قدم", "أقدام", "′"]),
    u("yd", Dim::Length, 0.9144, "ياردة", "yd", "m", &["yd", "yds", "yard", "yards", "ياردة", "يارد", "ياردات"]),
    u("mi", Dim::Length, 1609.344, "ميل", "mi", "km", &["mi", "mile", "miles", "ميل", "أميال", "ميلات"]),
    u("nmi", Dim::Length, 1852.0, "ميل بحري", "nmi", "km", &["nmi", "nautical mile", "nautical miles", "ميل بحري", "أميال بحرية"]),
    // Mass (kilogram).
    u("mg", Dim::Mass, 1e-6, "ملغ", "mg", "g", &["mg", "milligram", "milligrams", "ملغ", "مغ", "ملجم", "مليغرام", "مليجرام", "ملليغرام", "ملليجرام"]),
    u("g", Dim::Mass, 0.001, "غرام", "g", "oz", &["g", "gr", "gram", "grams", "gramme", "grammes", "غ", "غم", "جم", "غرام", "جرام", "غرامات", "جرامات"]),
    u("kg", Dim::Mass, 1.0, "كغ", "kg", "lb", &["kg", "kgs", "kilo", "kilos", "kilogram", "kilograms", "كغ", "كجم", "كيلو", "كيلوغرام", "كيلوجرام", "كيلو غرام", "كيلو جرام", "كيلوات", "كيلوغرامات", "كيلوجرامات"]),
    u("t", Dim::Mass, 1000.0, "طن", "t", "kg", &["t", "ton", "tons", "tonne", "tonnes", "طن", "أطنان"]),
    u("lb", Dim::Mass, 0.453_592_37, "رطل", "lb", "kg", &["lb", "lbs", "pound", "pounds", "رطل", "أرطال", "باوند", "پاوند", "باون"]),
    u("oz", Dim::Mass, 0.028_349_523_125, "أونصة", "oz", "g", &["oz", "ounce", "ounces", "أونصة", "اونصه", "أوقية", "أونس"]),
    // Temperature (kelvin), with offsets.
    Unit { id: "c", dim: Dim::Temperature, factor: 1.0, offset: 273.15, ar: "درجة مئوية", en: "°C", default_to: "f", aliases: &["c", "°c", "℃", "° c", "celsius", "centigrade", "deg c", "مئوي", "مئوية", "درجة مئوية", "درجات مئوية", "درجة", "درجات", "سيليزي", "سلسيوس", "سيلسيوس", "سيليزية", "°م", "م°"] },
    Unit { id: "f", dim: Dim::Temperature, factor: 5.0 / 9.0, offset: 459.67 * 5.0 / 9.0, ar: "فهرنهايت", en: "°F", default_to: "c", aliases: &["f", "°f", "℉", "° f", "fahrenheit", "deg f", "فهرنهايت", "فهرنهيت", "فرنهايت", "فارنهايت", "درجة فهرنهايت", "درجات فهرنهايت", "°ف"] },
    Unit { id: "k", dim: Dim::Temperature, factor: 1.0, offset: 0.0, ar: "كلفن", en: "K", default_to: "c", aliases: &["k", "kelvin", "كلفن", "كالفن", "كلڤن"] },
    // Area (m²).
    u("cm2", Dim::Area, 1e-4, "سم²", "cm²", "in2", &["cm2", "cm²", "sq cm", "square centimeter", "square centimeters", "سم2", "سم²", "سم مربع", "سنتيمتر مربع"]),
    u("in2", Dim::Area, 0.000_645_16, "إنش²", "in²", "cm2", &["in2", "in²", "sq in", "square inch", "square inches", "إنش مربع", "انش مربع", "بوصة مربعة"]),
    u("m2", Dim::Area, 1.0, "م²", "m²", "ft2", &["m2", "m²", "sq m", "sqm", "square meter", "square meters", "square metre", "square metres", "م2", "م²", "متر مربع", "أمتار مربعة", "متر مربع"]),
    u("ft2", Dim::Area, 0.092_903_04, "قدم²", "ft²", "m2", &["ft2", "ft²", "sq ft", "sqft", "square foot", "square feet", "قدم مربع", "أقدام مربعة"]),
    u("km2", Dim::Area, 1e6, "كم²", "km²", "mi2", &["km2", "km²", "sq km", "square kilometer", "square kilometers", "square kilometre", "square kilometres", "كم2", "كم²", "كم مربع", "كيلومتر مربع", "كيلو متر مربع"]),
    u("mi2", Dim::Area, 2_589_988.110_336, "ميل²", "mi²", "km2", &["mi2", "mi²", "sq mi", "square mile", "square miles", "ميل مربع", "أميال مربعة"]),
    u("ha", Dim::Area, 10_000.0, "هكتار", "ha", "acre", &["ha", "hectare", "hectares", "هكتار", "هكتارات"]),
    u("acre", Dim::Area, 4_046.856_422_4, "أكر", "ac", "ha", &["acre", "acres", "ac", "أكر", "إيكر", "ايكر"]),
    u("feddan", Dim::Area, 4_200.833, "فدان", "feddan", "m2", &["feddan", "feddans", "فدان", "أفدنة"]),
    u("dunam", Dim::Area, 1_000.0, "دونم", "dunam", "m2", &["dunam", "dunams", "donum", "دونم", "دونمات", "دنم"]),
    // Volume (litre).
    u("ml", Dim::Volume, 0.001, "مل", "ml", "floz", &["ml", "milliliter", "milliliters", "millilitre", "millilitres", "cc", "cm3", "cm³", "مل", "ملل", "مليلتر", "ملليلتر", "ميليلتر", "سم3", "سم³", "سم مكعب"]),
    u("l", Dim::Volume, 1.0, "لتر", "L", "gal", &["l", "liter", "liters", "litre", "litres", "ltr", "لتر", "لترات", "ليتر", "ليترات"]),
    u("m3", Dim::Volume, 1000.0, "م³", "m³", "l", &["m3", "m³", "cubic meter", "cubic meters", "cubic metre", "cubic metres", "م3", "م³", "متر مكعب", "أمتار مكعبة"]),
    u("gal", Dim::Volume, 3.785_411_784, "جالون", "gal", "l", &["gal", "gals", "gallon", "gallons", "جالون", "جالونات", "غالون", "غالونات"]),
    u("floz", Dim::Volume, 0.029_573_529_562_5, "أونصة سائلة", "fl oz", "ml", &["fl oz", "floz", "fl. oz", "fluid ounce", "fluid ounces", "أونصة سائلة", "أونصات سائلة"]),
    u("cup", Dim::Volume, 0.236_588_236_5, "كوب", "cup", "ml", &["cup", "cups", "كوب", "أكواب"]),
    // Data (byte), 1024-based as Windows shows sizes.
    u("bit", Dim::Data, 0.125, "بت", "bit", "b", &["bit", "bits", "بت"]),
    u("b", Dim::Data, 1.0, "بايت", "B", "bit", &["b", "byte", "bytes", "بايت", "بايتات"]),
    u("kb", Dim::Data, 1024.0, "كيلوبايت", "KB", "b", &["kb", "kib", "kilobyte", "kilobytes", "كيلوبايت", "كيلو بايت", "ك.ب"]),
    u("mb", Dim::Data, 1_048_576.0, "ميجابايت", "MB", "kb", &["mb", "mib", "mega", "megabyte", "megabytes", "ميجابايت", "ميغابايت", "ميقابايت", "ميجا بايت", "ميغا بايت", "ميجا", "ميغا", "ميقا", "م.ب"]),
    u("gb", Dim::Data, 1_073_741_824.0, "جيجابايت", "GB", "mb", &["gb", "gib", "giga", "gigabyte", "gigabytes", "gigs", "جيجابايت", "غيغابايت", "قيقابايت", "جيجا بايت", "غيغا بايت", "قيقا بايت", "جيجا", "غيغا", "قيقا", "جيغا", "ج.ب"]),
    u("tb", Dim::Data, 1_099_511_627_776.0, "تيرابايت", "TB", "gb", &["tb", "tib", "tera", "terabyte", "terabytes", "تيرابايت", "تيرا بايت", "تيرا", "ت.ب"]),
    u("pb", Dim::Data, 1_125_899_906_842_624.0, "بيتابايت", "PB", "tb", &["pb", "pib", "petabyte", "petabytes", "بيتابايت", "بيتا بايت", "بيتا"]),
    // Time (second).
    u("ms", Dim::Time, 0.001, "ملي ثانية", "ms", "s", &["ms", "millisecond", "milliseconds", "msec", "ملي ثانية", "مللي ثانية", "ميلي ثانية", "ملي ثانيه"]),
    u("s", Dim::Time, 1.0, "ثانية", "s", "min", &["s", "sec", "secs", "second", "seconds", "ث", "ثانية", "ثواني", "ثوان", "ثانيه"]),
    u("min", Dim::Time, 60.0, "دقيقة", "min", "s", &["min", "mins", "minute", "minutes", "د", "دقيقة", "دقائق", "دقيقه", "دقايق"]),
    u("h", Dim::Time, 3600.0, "ساعة", "h", "min", &["h", "hr", "hrs", "hour", "hours", "س", "ساعة", "ساعات", "ساعه"]),
    u("day", Dim::Time, DAY, "يوم", "d", "h", &["d", "day", "days", "يوم", "أيام", "يوما", "يومًا"]),
    u("week", Dim::Time, 7.0 * DAY, "أسبوع", "wk", "day", &["w", "wk", "wks", "week", "weeks", "أسبوع", "أسابيع"]),
    u("month", Dim::Time, 30.436_875 * DAY, "شهر", "mo", "day", &["month", "months", "mo", "شهر", "أشهر", "شهور"]),
    u("year", Dim::Time, 365.2425 * DAY, "سنة", "yr", "day", &["y", "yr", "yrs", "year", "years", "سنة", "سنوات", "سنين", "عام", "أعوام"]),
    // Speed (m/s).
    u("mps", Dim::Speed, 1.0, "م/ث", "m/s", "kmh", &["m/s", "mps", "meters per second", "metres per second", "م/ث", "متر/ث", "متر/ثانية", "متر في الثانية"]),
    u("kmh", Dim::Speed, 1000.0 / 3600.0, "كم/س", "km/h", "mph", &["km/h", "kmh", "kph", "km/hr", "kmph", "kilometers per hour", "kilometres per hour", "كم/س", "كم/ساعة", "كم في الساعة", "كيلومتر في الساعة", "كيلو متر في الساعة", "كيلومتر/ساعة", "كلم/س"]),
    u("mph", Dim::Speed, 0.44704, "ميل/س", "mph", "kmh", &["mph", "mi/h", "miles per hour", "mile per hour", "ميل/س", "ميل/ساعة", "ميل في الساعة"]),
    u("knot", Dim::Speed, 1852.0 / 3600.0, "عقدة", "kn", "kmh", &["knot", "knots", "kn", "kt", "kts", "عقدة", "عقد"]),
];

/// Words between the amount and the target unit: "to", "إلى", "=".
const CONNECTORS: &[&str] = &["to", "in", "into", "as", "=", "->", "→", "إلى", "الى", "في"];

/// Prefixes glued to an Arabic target unit: "بالباوند", "للميل", "بميل".
const GLUED: &[&str] = &["بال", "لل", "ال", "ب", "ل"];

/// Normalised alias → unit index, longest first.
fn alias_table() -> &'static [(String, usize)] {
    static T: OnceLock<Vec<(String, usize)>> = OnceLock::new();
    T.get_or_init(|| {
        let mut v: Vec<(String, usize)> = UNITS
            .iter()
            .enumerate()
            .flat_map(|(i, u)| u.aliases.iter().map(move |a| (norm(a), i)))
            .collect();
        v.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()).then_with(|| a.0.cmp(&b.0)));
        v.dedup();
        v
    })
}

/// Lower-case, Arabic-normalised text with ASCII digits and single spaces.
pub(crate) fn norm(s: &str) -> String {
    let n = crate::normalize::normalize(&calc::to_ascii(s));
    let mut out = String::with_capacity(n.len());
    for w in n.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// The unit named at the start of `s` (longest alias that ends at a word
/// boundary) and the rest of the text after it.
fn match_unit(s: &str) -> Option<(usize, &str, usize)> {
    for (alias, idx) in alias_table() {
        if let Some(rest) = s.strip_prefix(alias.as_str())
            && rest.chars().next().is_none_or(|c| !is_word_char(c))
        {
            return Some((*idx, rest.trim_start(), alias.chars().count()));
        }
    }
    None
}

/// The unit that is the whole of `s`, allowing a glued Arabic prefix.
fn whole_unit(s: &str) -> Option<usize> {
    let s = s.trim().trim_end_matches(['?', '؟']).trim_end();
    if s.is_empty() {
        return None;
    }
    if let Some((i, "", _)) = match_unit(s) {
        return Some(i);
    }
    for p in GLUED {
        if let Some(r) = s.strip_prefix(p)
            && let Some((i, "", _)) = match_unit(r.trim_start())
        {
            return Some(i);
        }
    }
    None
}

fn find(id: &str) -> Option<usize> {
    UNITS.iter().position(|u| u.id == id)
}

/// A parsed conversion.
#[derive(Debug, PartialEq)]
pub struct Conversion {
    pub value: f64,
    pub result: f64,
    from: usize,
    to: usize,
}

/// Reads "<number> <unit> [to <unit>]".
pub fn parse(query: &str) -> Option<Conversion> {
    if query.len() > 200 {
        return None;
    }
    let s = norm(query);
    let chars: Vec<char> = s.chars().collect();
    let mut pos = 0;
    let mut sign = 1.0;
    match chars.first() {
        Some('-' | '\u{2212}') => {
            sign = -1.0;
            pos = 1;
        }
        Some('+') => pos = 1,
        _ => {}
    }
    let value = match calc::read_number(&chars, &mut pos) {
        Some(Some(v)) => v * sign,
        _ => return None,
    };
    let rest: String = chars[pos..].iter().collect();
    let rest = rest.trim_start();
    let (from, after, alias_len) = match_unit(rest)?;
    let after = after.trim();
    let to = if after.is_empty() || after == "?" || after == "؟" {
        // "5 m" alone is too likely a file name; a one-letter unit needs a target.
        if alias_len < 2 {
            return None;
        }
        find(UNITS[from].default_to)?
    } else {
        let mut target = whole_unit(after);
        if target.is_none() {
            for c in CONNECTORS.iter().map(|c| norm(c)) {
                if let Some(r) = after.strip_prefix(c.as_str()) {
                    // A word connector must stand alone ("in cm", not "inch").
                    let word = c.chars().all(char::is_alphabetic);
                    if word && r.chars().next().is_some_and(is_word_char) {
                        continue;
                    }
                    if let Some(t) = whole_unit(r) {
                        target = Some(t);
                        break;
                    }
                }
            }
        }
        target?
    };
    if from == to || UNITS[from].dim != UNITS[to].dim {
        return None;
    }
    let (a, b) = (&UNITS[from], &UNITS[to]);
    let base = value * a.factor + a.offset;
    let result = (base - b.offset) / b.factor;
    if !result.is_finite() {
        return None;
    }
    Some(Conversion { value, result, from, to })
}

/// Title, copied text and an optional note for a conversion.
pub fn describe(c: &Conversion, english: bool, digits: Digits) -> (String, String, Option<String>) {
    let (a, b) = (&UNITS[c.from], &UNITS[c.to]);
    let name = |u: &Unit| if english { u.en } else { u.ar };
    let v = calc::localize(&calc::format_number(c.value, 10), digits);
    let r = calc::localize(&calc::format_number(c.result, 6), digits);
    let title = format!("{v} {} = {r} {}", name(a), name(b));
    let note = (a.dim == Dim::Data).then(|| {
        Lang::from_english(english).pick("1 كيلوبايت = 1024 بايت كما يعرضها ويندوز", "1 KB = 1024 bytes, as Windows shows sizes").to_string()
    });
    let note = note.map(|n| calc::localize(&n, digits));
    let copy = calc::localize(&calc::format_number(c.result, 6).replace(',', ""), digits);
    (title, copy, note)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(q: &str) -> Option<(f64, &'static str)> {
        parse(q).map(|c| (c.result, UNITS[c.to].id))
    }

    fn approx(q: &str, want: f64, to: &str) {
        let got = conv(q);
        let ok = got.is_some_and(|(v, id)| id == to && (v - want).abs() <= 1e-6 * want.abs().max(1.0));
        assert!(ok, "{q:?}: got {got:?}, want {want} {to}");
    }

    #[test]
    fn alias_table_is_sane() {
        for u in UNITS {
            assert!(find(u.default_to).is_some(), "{}", u.id);
            assert_eq!(UNITS[find(u.default_to).unwrap()].dim, u.dim, "{}", u.id);
            assert_ne!(u.default_to, u.id);
        }
        // No alias names two units of different dimensions.
        let t = alias_table();
        for (i, (a, x)) in t.iter().enumerate() {
            for (b, y) in &t[i + 1..] {
                if a == b {
                    panic!("alias {a:?} used by {} and {}", UNITS[*x].id, UNITS[*y].id);
                }
            }
        }
    }

    #[test]
    fn spec_examples() {
        approx("5 كيلو بالباوند", 11.023_113_109, "lb");
        approx("5 kg to lb", 11.023_113_109, "lb");
        approx("30 مئوي", 86.0, "f");
        approx("100 F to C", 37.777_777_78, "c");
        approx("10 كم بالميل", 6.213_711_922, "mi");
        approx("1 ميل", 1.609_344, "km");
        approx("2 جيجا بالميجا", 2048.0, "mb");
        approx("1 TB in GB", 1024.0, "gb");
        approx("3 ساعات بالدقائق", 180.0, "min");
        approx("100 كم/س بالميل/س", 62.137_119_22, "mph");
    }

    #[test]
    fn more_forms() {
        approx("5kg", 11.023_113_109, "lb");
        approx("10 lb", 4.535_923_7, "kg");
        approx("98.6 °F", 37.0, "c");
        approx("-40 c to f", -40.0, "f");
        approx("0 K to C", -273.15, "c");
        approx("٥ كيلوغرام", 11.023_113_109, "lb");
        approx("5 كيلوجرام إلى رطل", 11.023_113_109, "lb");
        approx("12 انش بالسم", 30.48, "cm");
        approx("6 قدم الى متر", 1.8288, "m");
        approx("2 ياردة بالمتر", 1.8288, "m");
        approx("1 لتر بالجالون", 0.264_172_052, "gal");
        approx("1 gallon to liters", 3.785_411_784, "l");
        approx("1 hectare to acres", 2.471_053_815, "acre");
        approx("1 فدان بالمتر مربع", 4200.833, "m2");
        approx("1 km2 in m2", 1e6, "m2");
        approx("90 ثانية بالدقائق", 1.5, "min");
        approx("2 days to hours", 48.0, "h");
        approx("1 week", 7.0, "day");
        approx("1 أسبوع بالأيام", 7.0, "day");
        approx("10 m/s to km/h", 36.0, "kmh");
        approx("1,000 m to km", 1.0, "km");
        approx("1.5 km = m", 1500.0, "m");
        approx("1 mile in km", 1.609_344, "km");
        approx("5 ft in cm", 152.4, "cm");
        approx("10 inch", 25.4, "cm");
        approx("512 MB to GB", 0.5, "gb");
        approx("8 bits to bytes", 1.0, "b");
        approx("1 طن بالكيلو", 1000.0, "kg");
        approx("500 غرام بالأونصة", 17.636_980_96, "oz");
        approx("37 درجة مئوية بالفهرنهايت", 98.6, "f");
        approx("30 درجة", 86.0, "f");
        approx("٢٫٥ كم بالمتر", 2500.0, "m");
        approx("20 knots to km/h", 37.04, "kmh");
        approx("100 km/h", 62.137_119_22, "mph");
        approx("5 kg to lb?", 11.023_113_109, "lb");
    }

    #[test]
    fn not_conversions() {
        for q in [
            "", "kg", "kg to lb", "5", "5 m", "5 g", "10 c", "5 kg to", "5 kg to km", "5 kg to kg", "5 kgx", "5 كيلوهات",
            "تقرير 5 kg", "5 kg report", "5 inchworm", "1.2.3 kg", "1,5 kg", "5 kg to lb extra", "😀 kg", "5 😀",
            "2026", "5 days ago", "1 ميلاد", "10 درجات الحرارة",
        ] {
            assert_eq!(conv(q), None, "{q:?}");
        }
    }

    #[test]
    fn describe_text() {
        let c = parse("5 kg to lb").unwrap();
        let (t, copy, note) = describe(&c, true, Digits::Ascii);
        assert_eq!(t, "5 kg = 11.0231 lb");
        assert_eq!(copy, "11.0231");
        assert!(note.is_none());
        let c = parse("٥ كيلو بالباوند").unwrap();
        let (t, copy, _) = describe(&c, false, Digits::Arabic);
        assert_eq!(t, "٥ كغ = ١١٫٠٢٣١ رطل");
        assert_eq!(copy, "١١٫٠٢٣١");
        let c = parse("2 جيجا بالميجا").unwrap();
        let (t, copy, note) = describe(&c, false, Digits::Ascii);
        assert_eq!(t, "2 جيجابايت = 2,048 ميجابايت");
        assert_eq!(copy, "2048");
        assert!(note.is_some());
    }

    #[test]
    fn huge_and_odd_values() {
        assert!(parse("1e400 kg").is_none());
        let big = format!("{} tb to b", "9".repeat(300));
        assert!(parse(&big).is_none());
        let _ = parse(&"9".repeat(5000));
        let _ = parse("5 \u{200F}kg\u{200F} to lb");
    }
}
