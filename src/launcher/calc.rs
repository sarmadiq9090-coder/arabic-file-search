//! The calculator behind the instant answers: a small recursive-descent
//! parser (no `eval`), plus the number reading and formatting shared with the
//! unit and date answers (Arabic-Indic and Persian digits, Arabic separators).

/// Which digits the user typed, so that answers come back in the same ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Digits {
    Ascii,
    /// ٠١٢٣٤٥٦٧٨٩ with ٫ and ٬.
    Arabic,
    /// ۰۱۲۳۴۵۶۷۸۹ with ٫ and ٬.
    Persian,
}

/// The digits used in `s`: Arabic-Indic or Persian if any appear, ASCII otherwise.
pub fn detect_digits(s: &str) -> Digits {
    for c in s.chars() {
        match c {
            '\u{0660}'..='\u{0669}' => return Digits::Arabic,
            '\u{06F0}'..='\u{06F9}' => return Digits::Persian,
            _ => {}
        }
    }
    Digits::Ascii
}

/// `s` with Arabic-Indic and Persian digits turned into ASCII ones, the
/// Arabic decimal separator `٫` into `.` and the Arabic thousands separator
/// `٬` into `,`.
pub fn to_ascii(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{0660}'..='\u{0669}' => char::from_u32(c as u32 - 0x0660 + '0' as u32).unwrap_or(c),
            '\u{06F0}'..='\u{06F9}' => char::from_u32(c as u32 - 0x06F0 + '0' as u32).unwrap_or(c),
            '\u{066B}' => '.',
            '\u{066C}' => ',',
            _ => c,
        })
        .collect()
}

/// Turns the ASCII digits and separators of a formatted number (or any text
/// holding such numbers) into the given digit style.
pub fn localize(s: &str, digits: Digits) -> String {
    let zero = match digits {
        Digits::Ascii => return s.to_string(),
        Digits::Arabic => 0x0660,
        Digits::Persian => 0x06F0,
    };
    let mut out = String::with_capacity(s.len() * 2);
    let mut prev_digit = false;
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let next_digit = chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        match c {
            '0'..='9' => {
                out.push(char::from_u32(zero + (c as u32 - '0' as u32)).unwrap_or(c));
                prev_digit = true;
                continue;
            }
            // Separators only between digits, so that "e.g." stays as it is.
            '.' if prev_digit && next_digit => out.push('\u{066B}'),
            ',' if prev_digit && next_digit => out.push('\u{066C}'),
            _ => out.push(c),
        }
        prev_digit = false;
    }
    out
}

/// Formats `v` with at most `sig` significant digits (and at most 10
/// decimals), trailing zeros trimmed and the integer part grouped by
/// thousands ("1,234.5"). Very large or very small values use the
/// scientific form ("1.5e20"). ASCII digits; see [`localize`].
pub fn format_number(v: f64, sig: i32) -> String {
    if !v.is_finite() {
        return String::new();
    }
    if v == 0.0 {
        return "0".to_string();
    }
    let abs = v.abs();
    if !(1e-9..1e15).contains(&abs) {
        return scientific(v, sig);
    }
    let int_digits = abs.log10().floor() as i32 + 1;
    let decimals = (sig - int_digits).clamp(0, 10) as usize;
    let s = format!("{:.*}", decimals, abs);
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f.trim_end_matches('0')),
        None => (s.as_str(), ""),
    };
    if int_part.bytes().all(|b| b == b'0') && frac_part.is_empty() {
        // Rounded away entirely (e.g. 1e-9 with few significant digits).
        return scientific(v, sig);
    }
    let mut out = String::new();
    if v < 0.0 {
        out.push('-');
    }
    out.push_str(&group_thousands(int_part));
    if !frac_part.is_empty() {
        out.push('.');
        out.push_str(frac_part);
    }
    out
}

fn scientific(v: f64, sig: i32) -> String {
    let prec = (sig - 1).clamp(0, 15) as usize;
    let s = format!("{:.*e}", prec, v);
    match s.split_once('e') {
        Some((mant, exp)) => {
            let mant = if mant.contains('.') { mant.trim_end_matches('0').trim_end_matches('.') } else { mant };
            format!("{mant}e{exp}")
        }
        None => s,
    }
}

/// "1234567" → "1,234,567".
pub fn group_thousands(int_part: &str) -> String {
    let n = int_part.len();
    let mut out = String::with_capacity(n + n / 3);
    for (i, c) in int_part.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Reads a number at the start of `chars[*pos..]` (ASCII digits, optional
/// "," thousands groups of exactly three digits, optional "." decimals).
/// Returns `None` without moving when there is no number there, and
/// `Some(None)` for a malformed one ("1,5", "1.2.3").
pub fn read_number(chars: &[char], pos: &mut usize) -> Option<Option<f64>> {
    let start = *pos;
    let mut i = start;
    let mut text = String::new();
    let mut int_digits = 0usize;
    while i < chars.len() && chars[i].is_ascii_digit() {
        text.push(chars[i]);
        int_digits += 1;
        i += 1;
    }
    // Thousands groups: ",ddd" right after 1–3 digits (or after another group).
    if int_digits > 0 && i < chars.len() && chars[i] == ',' {
        if int_digits > 3 {
            *pos = i;
            return Some(None);
        }
        while i < chars.len() && chars[i] == ',' {
            let group: Vec<char> = chars[i + 1..].iter().take_while(|c| c.is_ascii_digit()).copied().collect();
            if group.len() != 3 {
                *pos = i;
                return Some(None);
            }
            text.extend(group);
            i += 4;
        }
    }
    let mut frac_digits = 0usize;
    if i < chars.len() && chars[i] == '.' {
        let mut j = i + 1;
        while j < chars.len() && chars[j].is_ascii_digit() {
            frac_digits += 1;
            j += 1;
        }
        if frac_digits > 0 {
            text.push('.');
            text.extend(&chars[i + 1..j]);
            i = j;
        } else if int_digits > 0 {
            // "5." is five.
            i += 1;
        }
    }
    if int_digits == 0 && frac_digits == 0 {
        return None;
    }
    // A second decimal point ("1.2.3") is not a number.
    if i < chars.len() && (chars[i] == '.' || chars[i] == ',') && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
        *pos = i;
        return Some(None);
    }
    *pos = i;
    Some(text.parse::<f64>().ok().filter(|v| v.is_finite()))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Plus,
    Minus,
    Mul,
    Div,
    Pow,
    Percent,
    LParen,
    RParen,
    Sqrt,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = s.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            match read_number(&chars, &mut i) {
                Some(Some(v)) => toks.push(Tok::Num(v)),
                _ => return None,
            }
            continue;
        }
        let tok = match c {
            '+' => Tok::Plus,
            '-' | '\u{2212}' | '\u{2013}' => Tok::Minus,
            '*' | '×' | 'x' | 'X' | '\u{00B7}' | '\u{22C5}' => Tok::Mul,
            '/' | '÷' | '\u{2215}' => Tok::Div,
            '^' => Tok::Pow,
            '%' | '\u{066A}' => Tok::Percent,
            '(' | '[' => Tok::LParen,
            ')' | ']' => Tok::RParen,
            '√' => Tok::Sqrt,
            _ => {
                // Words: sqrt / جذر.
                let rest: String = chars[i..].iter().take(4).collect();
                if rest.eq_ignore_ascii_case("sqrt") {
                    i += 4;
                    toks.push(Tok::Sqrt);
                    continue;
                }
                let rest3: String = chars[i..].iter().take(3).collect();
                if rest3 == "جذر" {
                    i += 3;
                    toks.push(Tok::Sqrt);
                    continue;
                }
                return None;
            }
        };
        toks.push(tok);
        i += 1;
    }
    Some(toks)
}

const MAX_DEPTH: u32 = 64;

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: u32,
    /// A binary operator, a function, a percent or a power was seen.
    has_op: bool,
}

type R = Result<f64, ()>;

fn finite(v: f64) -> R {
    if v.is_finite() { Ok(v) } else { Err(()) }
}

impl Parser {
    fn peek(&self) -> Option<Tok> {
        self.toks.get(self.pos).copied()
    }

    fn peek_at(&self, k: usize) -> Option<Tok> {
        self.toks.get(self.pos + k).copied()
    }

    fn starts_operand(t: Option<Tok>) -> bool {
        matches!(t, Some(Tok::Num(_) | Tok::LParen | Tok::Sqrt | Tok::Minus | Tok::Plus))
    }

    fn enter(&mut self) -> Result<(), ()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(()) } else { Ok(()) }
    }

    /// expr := term (('+' | '-') term)*, where "a + b%" means a × (1 + b/100).
    fn expr(&mut self) -> R {
        self.enter()?;
        let (mut acc, _) = self.term()?;
        while let Some(op @ (Tok::Plus | Tok::Minus)) = self.peek() {
            self.pos += 1;
            self.has_op = true;
            let (rhs, pct) = self.term()?;
            let rhs = if pct { acc * rhs } else { rhs };
            acc = finite(if op == Tok::Plus { acc + rhs } else { acc - rhs })?;
        }
        self.depth -= 1;
        Ok(acc)
    }

    /// term := unary (('*' | '/' | '%') unary)*. Returns whether the term is a
    /// lone percentage ("10%"), for the "+ 10%" rule.
    fn term(&mut self) -> Result<(f64, bool), ()> {
        let (mut acc, mut pct) = self.unary()?;
        loop {
            match self.peek() {
                Some(Tok::Mul) => {
                    self.pos += 1;
                    self.has_op = true;
                    let (rhs, _) = self.unary()?;
                    acc = finite(acc * rhs)?;
                }
                Some(Tok::Div) => {
                    self.pos += 1;
                    self.has_op = true;
                    let (rhs, _) = self.unary()?;
                    if rhs == 0.0 {
                        return Err(());
                    }
                    acc = finite(acc / rhs)?;
                }
                // "10 % 3": modulo (a percent sign followed by an operand).
                Some(Tok::Percent) if Self::starts_operand(self.peek_at(1)) => {
                    self.pos += 1;
                    self.has_op = true;
                    let (rhs, _) = self.unary()?;
                    if rhs == 0.0 {
                        return Err(());
                    }
                    acc = finite(acc % rhs)?;
                }
                _ => break,
            }
            pct = false;
        }
        Ok((acc, pct))
    }

    /// unary := ('-' | '+') unary | power
    fn unary(&mut self) -> Result<(f64, bool), ()> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.pos += 1;
                self.enter()?;
                let (v, pct) = self.unary()?;
                self.depth -= 1;
                Ok((-v, pct))
            }
            Some(Tok::Plus) => {
                self.pos += 1;
                self.enter()?;
                let r = self.unary()?;
                self.depth -= 1;
                Ok(r)
            }
            _ => self.power(),
        }
    }

    /// power := postfix ('^' unary)?  (right-associative: 2^3^2 = 2^9)
    fn power(&mut self) -> Result<(f64, bool), ()> {
        let (base, pct) = self.postfix()?;
        if self.peek() == Some(Tok::Pow) {
            self.pos += 1;
            self.has_op = true;
            self.enter()?;
            let (exp, _) = self.unary()?;
            self.depth -= 1;
            if base == 0.0 && exp < 0.0 {
                return Err(());
            }
            let v = base.powf(exp);
            return Ok((finite(v)?, false));
        }
        Ok((base, pct))
    }

    /// postfix := primary '%'*   (a percent sign not followed by an operand)
    fn postfix(&mut self) -> Result<(f64, bool), ()> {
        let mut v = self.primary()?;
        let mut pct = false;
        while self.peek() == Some(Tok::Percent) && !Self::starts_operand(self.peek_at(1)) {
            self.pos += 1;
            self.has_op = true;
            v /= 100.0;
            pct = true;
        }
        Ok((v, pct))
    }

    /// primary := number | '(' expr ')' | sqrt postfix
    fn primary(&mut self) -> R {
        match self.peek() {
            Some(Tok::Num(v)) => {
                self.pos += 1;
                Ok(v)
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let v = self.expr()?;
                if self.peek() != Some(Tok::RParen) {
                    return Err(());
                }
                self.pos += 1;
                Ok(v)
            }
            Some(Tok::Sqrt) => {
                self.pos += 1;
                self.has_op = true;
                self.enter()?;
                // √-4 is not real; √(…) and √16 are fine.
                let (v, _) = self.postfix()?;
                self.depth -= 1;
                if v < 0.0 {
                    return Err(());
                }
                Ok(v.sqrt())
            }
            _ => Err(()),
        }
    }
}

/// Text that looks like a date, a phone number or a version rather than a
/// sum: "2026-03-20", "1/9/1447", "050-123-4567", "1.2.3" (the last one is
/// already refused by the number reader).
fn looks_like_date_or_serial(s: &str) -> bool {
    let t = s.trim();
    if t.chars().any(char::is_whitespace) {
        return false;
    }
    for sep in ['-', '/', '.'] {
        let parts: Vec<&str> = t.split(sep).collect();
        if parts.len() >= 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
            return true;
        }
    }
    false
}

/// Evaluates `query` as arithmetic. `None` unless it holds at least one
/// operator and parses completely to a finite value (no division by zero).
pub fn evaluate(query: &str) -> Option<f64> {
    if query.len() > 512 {
        return None;
    }
    let ascii = to_ascii(query);
    let mut s = ascii.trim();
    s = s.strip_prefix('=').unwrap_or(s).trim();
    s = s.strip_suffix('=').unwrap_or(s).trim();
    if s.is_empty() || looks_like_date_or_serial(s) {
        return None;
    }
    let toks = tokenize(s)?;
    let mut p = Parser { toks, pos: 0, depth: 0, has_op: false };
    let v = p.expr().ok()?;
    if p.pos != p.toks.len() || !p.has_op || !v.is_finite() {
        return None;
    }
    Some(if v == 0.0 { 0.0 } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Option<f64> {
        evaluate(s)
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() <= 1e-9 * b.abs().max(1.0))
    }

    #[test]
    fn basic_arithmetic() {
        assert!(close(ev("1+2"), 3.0));
        assert!(close(ev("2+3*4"), 14.0));
        assert!(close(ev("(2+3)*4"), 20.0));
        assert!(close(ev("10/4"), 2.5));
        assert!(close(ev("2^10"), 1024.0));
        assert!(close(ev("2^3^2"), 512.0));
        assert!(close(ev("-2^2"), -4.0));
        assert!(close(ev("2^-1"), 0.5));
        assert!(close(ev("-(3+2)*2"), -10.0));
        assert!(close(ev("2*-3"), -6.0));
        assert!(close(ev(" 7 - 2 - 1 "), 4.0));
        assert!(close(ev("1.5*2"), 3.0));
        assert!(close(ev(".5+.25"), 0.75));
        assert!(close(ev("1500 × 2"), 3000.0));
        assert!(close(ev("9 ÷ 3"), 3.0));
        assert!(close(ev("3x4"), 12.0));
        assert!(close(ev("2+2="), 4.0));
        assert!(close(ev("=2+2"), 4.0));
        assert!(close(ev("[1+1]*3"), 6.0));
        assert!(close(ev("5 − 2"), 3.0));
    }

    #[test]
    fn percent_and_modulo() {
        assert!(close(ev("50%"), 0.5));
        assert!(close(ev("200*10%"), 20.0));
        assert!(close(ev("200+10%"), 220.0));
        assert!(close(ev("200-10%"), 180.0));
        assert!(close(ev("10%3"), 1.0));
        assert!(close(ev("10 % 4"), 2.0));
        assert!(close(ev("50٪"), 0.5));
        assert_eq!(ev("10%0"), None);
    }

    #[test]
    fn sqrt_forms() {
        assert!(close(ev("sqrt(16)"), 4.0));
        assert!(close(ev("SQRT 9"), 3.0));
        assert!(close(ev("√16"), 4.0));
        assert!(close(ev("√(9+16)"), 5.0));
        assert!(close(ev("جذر 25"), 5.0));
        assert!(close(ev("جذر(٨١)"), 9.0));
        assert_eq!(ev("2√9"), None); // no implicit multiplication
        assert_eq!(ev("sqrt(-4)"), None);
        assert_eq!(ev("√-4"), None);
    }

    #[test]
    fn arabic_and_persian_digits() {
        assert!(close(ev("١٢+٣"), 15.0));
        assert!(close(ev("۱۲*۲"), 24.0));
        assert!(close(ev("٢٫٥×٢"), 5.0));
        assert!(close(ev("١٬٠٠٠+١"), 1001.0));
        assert!(close(ev("1,000+1"), 1001.0));
        assert!(close(ev("1,234,567*1"), 1234567.0));
    }

    #[test]
    fn needs_an_operator() {
        assert_eq!(ev("2026"), None);
        assert_eq!(ev("-5"), None);
        assert_eq!(ev("+5"), None);
        assert_eq!(ev("(5)"), None);
        assert_eq!(ev("1,000"), None);
        assert_eq!(ev("٣"), None);
    }

    #[test]
    fn malformed_is_none() {
        for s in [
            "", " ", "5-", "((", "()", "(1+2", "1+2)", "+", "*", "1**2", "1,5+1", "1.2.3+1", "1234,567+1", "abc",
            "1+a", "2+3 هجري", "تقرير 2+3", "😀+1", "1+😀", "%", "%5", "^2", "2^", "√", "sqrt", "جذر",
            "1/9/1447", "2026-03-20", "2026/3/20", "050-123-4567", "1.2.3", "1 2", "5 5+1",
        ] {
            assert_eq!(ev(s), None, "{s:?}");
        }
    }

    #[test]
    fn division_by_zero_and_overflow() {
        assert_eq!(ev("0/0"), None);
        assert_eq!(ev("1/0"), None);
        assert_eq!(ev("5/(2-2)"), None);
        assert_eq!(ev("10^400"), None);
        assert_eq!(ev("0^-1"), None);
        assert_eq!(ev("99999999999999999999999999999999^99"), None);
        assert_eq!(ev("1e3"), None);
    }

    #[test]
    fn deep_nesting_does_not_overflow() {
        let deep = format!("{}1+1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert_eq!(ev(&deep), None);
        let minus = format!("1{}1", "-".repeat(10_000));
        assert_eq!(ev(&minus), None);
        let ok = format!("{}1+1{}", "(".repeat(30), ")".repeat(30));
        assert!(close(ev(&ok), 2.0));
    }

    #[test]
    fn formatting() {
        assert_eq!(format_number(3000.0, 12), "3,000");
        assert_eq!(format_number(0.1 + 0.2, 12), "0.3");
        assert_eq!(format_number(2.0 / 3.0, 12), "0.6666666667");
        assert_eq!(format_number(-1234.5, 12), "-1,234.5");
        assert_eq!(format_number(0.0, 12), "0");
        assert_eq!(format_number(1e20, 12), "1e20");
        assert_eq!(format_number(1.5e-12, 12), "1.5e-12");
        assert_eq!(format_number(123456789.0, 12), "123,456,789");
        assert_eq!(format_number(11.023113109, 6), "11.0231");
        assert_eq!(format_number(999.9999999, 6), "1,000");
        assert_eq!(format_number(f64::NAN, 6), "");
        assert_eq!(group_thousands("1"), "1");
        assert_eq!(group_thousands("123"), "123");
        assert_eq!(group_thousands("1234"), "1,234");
    }

    #[test]
    fn localizing() {
        assert_eq!(detect_digits("١+٢"), Digits::Arabic);
        assert_eq!(detect_digits("۱+۲"), Digits::Persian);
        assert_eq!(detect_digits("1+2"), Digits::Ascii);
        assert_eq!(localize("3,000.5", Digits::Arabic), "٣٬٠٠٠٫٥");
        assert_eq!(localize("3,000.5", Digits::Persian), "۳٬۰۰۰٫۵");
        assert_eq!(localize("3,000.5", Digits::Ascii), "3,000.5");
        assert_eq!(localize("5 kg. ok", Digits::Arabic), "٥ kg. ok");
    }

    #[test]
    fn random_input_never_panics() {
        let alphabet: Vec<char> = "0123456789+-*/×÷^%()., ٠١٢٣٤٥٦٧٨٩٫٬√جذرsqrtx😀\u{200F}=[]e".chars().collect();
        let mut seed = 0x2545F4914F6CDD1Du64;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed % 14) as usize;
            let mut s = String::new();
            let mut x = seed;
            for _ in 0..len {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                s.push(alphabet[((x >> 33) % alphabet.len() as u64) as usize]);
            }
            if let Some(v) = evaluate(&s) {
                assert!(v.is_finite(), "{s:?}");
                let _ = format_number(v, 12);
            }
        }
    }
}
