//! Instant answers: calculator, unit conversion, currency conversion, Hijri
//! and Gregorian dates.

use super::{Action, Context, Item, Kind, calc, currency, hijri, units};
use crate::i18n::Lang;

/// Score of an instant answer: it is exactly what was asked, so it leads.
pub const ANSWER_SCORE: u32 = 1000;

fn copy_hint(english: bool) -> &'static str {
    Lang::from_english(english).pick("اضغط Enter للنسخ", "Press Enter to copy")
}

fn item(title: String, subtitle: String, copy: String) -> Item {
    Item { kind: Kind::Answer, title, subtitle, action: Action::CopyText(copy), score: ANSWER_SCORE, icon_path: None, strong: false }
}

/// Where the downloaded exchange rates come from. Their free terms ask for
/// this attribution, with a link, wherever the rates are shown.
pub const RATES_CREDIT: &str = "Rates By Exchange Rate API";
/// The link that goes with [`RATES_CREDIT`].
pub const RATES_CREDIT_URL: &str = "https://www.exchangerate-api.com";

/// The row under every currency answer: the attribution, opening the
/// provider's site on a click. It is a source line, not a second answer:
/// the arrow keys pass over it (`crate::quick::selectable`), so Enter always
/// copies the amount. The credit itself stays in English (it is their
/// wording); the line under it is in the interface language.
pub fn rates_credit(english: bool) -> Item {
    let subtitle = Lang::from_english(english)
        .pick("مصدر أسعار الصرف · انقر لفتح موقع exchangerate-api.com", "Exchange-rate source · click to open exchangerate-api.com");
    Item {
        kind: Kind::Answer,
        title: RATES_CREDIT.to_string(),
        subtitle: subtitle.to_string(),
        action: Action::OpenUri(RATES_CREDIT_URL.to_string()),
        score: ANSWER_SCORE - 1,
        icon_path: None,
        strong: false,
    }
}

/// The instant answers for `query`: [`answer`], followed by the rates
/// attribution ([`rates_credit`]) when the answer is a currency conversion.
pub fn answers(query: &str, ctx: &Context) -> Vec<Item> {
    let Some(a) = answer(query, ctx) else { return Vec::new() };
    let money = is_currency(query.trim(), ctx);
    let mut v = vec![a];
    if money {
        v.push(rates_credit(ctx.english));
    }
    v
}

/// True when [`answer`] answers `q` with a currency conversion (the same
/// order of checks: dates and units win over money).
fn is_currency(q: &str, ctx: &Context) -> bool {
    !q.is_empty()
        && q.len() <= 512
        && hijri::answer(q, ctx.today, ctx.english).is_none()
        && units::parse(q).is_none()
        && currency::answer(q, ctx.english, &currency::current()).is_some()
}

/// An instant answer for `query`, if it is a calculation, a conversion or a date question.
pub fn answer(query: &str, ctx: &Context) -> Option<Item> {
    let q = query.trim();
    if q.is_empty() || q.len() > 512 {
        return None;
    }
    let hint = copy_hint(ctx.english);
    // Dates first: "1/9/1447 هجري" is a date, not a division.
    if let Some((title, copy, note)) = hijri::answer(q, ctx.today, ctx.english) {
        return Some(item(title, format!("{note} · {hint}"), copy));
    }
    let digits = calc::detect_digits(q);
    if let Some(c) = units::parse(q) {
        let (title, copy, note) = units::describe(&c, ctx.english, digits);
        let subtitle = match note {
            Some(n) => format!("{n} · {hint}"),
            None => hint.to_string(),
        };
        return Some(item(title, subtitle, copy));
    }
    // After the units: "100 pounds" is a weight; "100 gbp" is money.
    if let Some((title, copy, note)) = currency::answer(q, ctx.english, &currency::current()) {
        // "approximate rate · 2026-11-10 · Enter copies"
        let hint = Lang::from_english(ctx.english).pick("Enter للنسخ", "Enter copies");
        return Some(item(title, format!("{note} · {hint}"), copy));
    }
    let v = calc::evaluate(q)?;
    let shown = calc::format_number(v, 12);
    let title = calc::localize(&format!("= {shown}"), digits);
    let copy = calc::localize(&shown.replace(',', ""), digits);
    Some(item(title, hint.to_string(), copy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(english: bool) -> Context<'static> {
        static NO_LAUNCHES: fn(&str) -> u32 = |_| 0;
        Context { english, windows_build: 0, windows: &[], launches: &NO_LAUNCHES, picked: &NO_LAUNCHES, today: (2026, 11, 1) }
    }

    fn title(q: &str) -> Option<String> {
        answer(q, &ctx(false)).map(|i| i.title)
    }

    #[test]
    fn calculator_answers() {
        let i = answer("1500*2", &ctx(false)).unwrap();
        assert_eq!(i.kind, Kind::Answer);
        assert_eq!(i.title, "= 3,000");
        assert_eq!(i.subtitle, "اضغط Enter للنسخ");
        assert_eq!(i.action, Action::CopyText("3000".into()));
        assert_eq!(answer("1500*2", &ctx(true)).unwrap().subtitle, "Press Enter to copy");
        assert_eq!(title("١٥٠٠×٢").as_deref(), Some("= ٣٬٠٠٠"));
        assert_eq!(answer("١٥٠٠×٢", &ctx(false)).unwrap().action, Action::CopyText("٣٠٠٠".into()));
        assert_eq!(title("۱۰÷۴").as_deref(), Some("= ۲٫۵"));
        assert_eq!(title("0.1+0.2").as_deref(), Some("= 0.3"));
        assert_eq!(title("-5*3").as_deref(), Some("= -15"));
        assert_eq!(title("sqrt(2)").as_deref(), Some("= 1.4142135624"));
    }

    #[test]
    fn non_answers() {
        for q in ["", "  ", "2026", "تقرير", "report.pdf", "5-", "((", "0/0", "1/0", "😀", "\u{200F}", "ملف 2+2", "v1.2.3"] {
            assert_eq!(title(q), None, "{q:?}");
        }
    }

    #[test]
    fn unit_and_date_answers() {
        assert_eq!(title("5 kg to lb").as_deref(), Some("5 كغ = 11.0231 رطل"));
        assert_eq!(answer("5 kg to lb", &ctx(true)).unwrap().title, "5 kg = 11.0231 lb");
        let d = answer("1/9/1447 هجري", &ctx(false)).unwrap();
        assert_eq!(d.title, "1 رمضان 1447 هـ = الأربعاء 18 فبراير 2026 م");
        assert_eq!(d.subtitle, "تقويم أم القرى · اضغط Enter للنسخ");
        assert_eq!(title("هجري").as_deref(), Some("21 جمادى الأولى 1448 هـ"));
        let g = answer("2 GB to MB", &ctx(true)).unwrap();
        assert!(g.subtitle.contains("1024"), "{}", g.subtitle);
    }

    #[test]
    fn currency_answers() {
        let i = answer("100 usd to iqd", &ctx(true)).unwrap();
        assert_eq!(i.kind, Kind::Answer);
        assert!(i.title.starts_with("100 USD = ") && i.title.ends_with(" IQD"), "{}", i.title);
        assert!(i.subtitle.starts_with("approximate ") && i.subtitle.ends_with(" · Enter copies"), "{}", i.subtitle);
        assert!(matches!(&i.action, Action::CopyText(t) if !t.contains(',') && t.parse::<f64>().is_ok()));
        let i = answer("١٠٠ دولار", &ctx(false)).unwrap();
        assert!(i.title.starts_with("١٠٠ دولار = ") && i.title.ends_with(" دينار عراقي"), "{}", i.title);
        assert!(i.subtitle.contains("يورو") && i.subtitle.contains("سعر تقريبي") && i.subtitle.ends_with(" · Enter للنسخ"), "{}", i.subtitle);
        for q in ["50 يورو بالدينار", "100$ to iqd", "1000 iqd usd", "100 ريال سعودي", "100 ليرة تركية"] {
            assert!(answer(q, &ctx(false)).is_some(), "{q}");
        }
        // Not money: plain searches, and what the calculator and units answer.
        for q in ["100 files", "usd", "report 2024", "دولار"] {
            assert_eq!(title(q), None, "{q}");
        }
        assert_eq!(title("2+2").as_deref(), Some("= 4"));
        assert_eq!(answer("5 km to miles", &ctx(true)).unwrap().title, "5 km = 3.10686 mi");
        assert!(answer("100 pounds", &ctx(true)).unwrap().title.contains("kg"));
    }

    #[test]
    fn currency_answers_carry_the_rates_credit() {
        for (q, english) in [("100 usd to iqd", true), ("١٠٠ دولار", false), ("50 يورو بالدينار", false)] {
            let v = answers(q, &ctx(english));
            assert_eq!(v.len(), 2, "{q}");
            assert_eq!(v[1].title, "Rates By Exchange Rate API");
            assert_eq!(v[1].action, Action::OpenUri("https://www.exchangerate-api.com".into()));
            assert!(v[1].score < v[0].score);
            let arabic = v[1].subtitle.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
            assert_eq!(arabic, !english, "{}", v[1].subtitle);
        }
        // Not money: no credit.
        for q in ["2+2", "5 kg to lb", "100 pounds", "هجري", "تقرير", ""] {
            assert!(answers(q, &ctx(true)).iter().all(|i| i.title != RATES_CREDIT), "{q}");
        }
    }

    #[test]
    fn mixed_and_random_input_never_panics() {
        let samples = [
            "٥ + ٣ ملفات", "abc ١٢٣", "\u{202B}5+5\u{202C}", "5 كم\u{200F} بالميل", "🙂🙂🙂", "١/٩/١٤٤٧ هـ", "%%%%", "--5",
            "(((((((((", ")))))", "5 kg to lb to oz", "٫", "٬", ",", ".", "1,", "1.", "x", "×", "√√√√", "جذر جذر",
        ];
        for s in samples {
            let _ = answer(s, &ctx(false));
            let _ = answer(s, &ctx(true));
        }
        let pieces = ["1", "٢", "+", "-", "/", "kg", "كم", "بال", "ميل", "هجري", "/", " ", "(", ")", "%", "رمضان", "😀", "1447", "to", "."];
        let mut x = 12345u64;
        for _ in 0..20_000 {
            let mut s = String::new();
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            for k in 0..((x >> 61) + 1) {
                let idx = ((x >> (k * 5 + 3)) % pieces.len() as u64) as usize;
                s.push_str(pieces[idx]);
            }
            let _ = answer(&s, &ctx(false));
        }
    }
}
