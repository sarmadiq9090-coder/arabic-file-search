//! Instant answers: calculator, unit conversion, currency conversion, Hijri
//! and Gregorian dates.

use super::{Action, Context, Item, Kind, calc, currency, hijri, units};

/// Score of an instant answer: it is exactly what was asked, so it leads.
pub const ANSWER_SCORE: u32 = 1000;

fn copy_hint(english: bool) -> &'static str {
    if english { "Press Enter to copy" } else { "اضغط Enter للنسخ" }
}

fn item(title: String, subtitle: String, copy: String) -> Item {
    Item { kind: Kind::Answer, title, subtitle, action: Action::CopyText(copy), score: ANSWER_SCORE, icon_path: None, strong: false }
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
        assert!(i.subtitle.ends_with("Press Enter to copy"), "{}", i.subtitle);
        assert!(matches!(&i.action, Action::CopyText(t) if !t.contains(',') && t.parse::<f64>().is_ok()));
        let i = answer("١٠٠ دولار", &ctx(false)).unwrap();
        assert!(i.title.starts_with("١٠٠ دولار = ") && i.title.ends_with(" دينار عراقي"), "{}", i.title);
        assert!(i.subtitle.contains("يورو") && i.subtitle.contains("سعر"), "{}", i.subtitle);
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
