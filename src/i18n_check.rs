//! The list of English texts to translate, and the check of the language
//! files against it (`cargo test i18n -- --nocapture` prints the table).
//!
//! The texts are read from the sources themselves: every `tr!`/`trf!`
//! with two literals (`trf!(in lang; …)` too), every `.pick("…", "…")`,
//! every `("عربي", "English")` pair in a tuple, the `*_EN` arrays next to
//! their Arabic ones (`NAME` or `NAME_AR`), and the `T` table of
//! `src/webui/assets/app.js`. `I18N_WRITE_SOURCE=1 cargo test i18n_source`
//! writes them to `i18n/source.json`, the file translators work from.
//!
//! A language file fails the test when it is not valid JSON, when its
//! `meta` disagrees with its row in [`super::LOCALES`], or when a
//! translation's placeholders differ from the English text's (`trf!`
//! templates, and app.js functions, whose translations must be
//! expressions of the small language app.js understands). Missing
//! translations and keys whose English text changed are reported, not
//! failed: a missing one reads English.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ------------------------------------------------------------------ Rust lexer

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Str(String),
    Ident(String),
    Punct(char),
}

/// Rust source as tokens: comments dropped, string literals unescaped,
/// byte strings and chars dropped.
fn lex(src: &str) -> Vec<Tok> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if ch == '"' {
            let (s, end) = string(&c, i + 1);
            out.push(Tok::Str(s));
            i = end;
        } else if (ch == 'r' || ch == 'b') && is_raw_start(&c, i) {
            let byte = ch == 'b';
            let mut j = i + if byte { 1 } else { 0 };
            if c.get(j) == Some(&'r') {
                j += 1;
                let hashes = c[j..].iter().take_while(|&&x| x == '#').count();
                j += hashes + 1;
                let start = j;
                loop {
                    if j >= c.len() {
                        break;
                    }
                    if c[j] == '"' && c[j + 1..].iter().take(hashes).filter(|&&x| x == '#').count() == hashes && c[j + 1..].len() >= hashes {
                        break;
                    }
                    j += 1;
                }
                let s: String = c[start..j.min(c.len())].iter().collect();
                if !byte {
                    out.push(Tok::Str(s));
                }
                i = j + 1 + hashes;
            } else {
                // b"…"
                let (_, end) = string(&c, j + 1);
                i = end;
            }
        } else if ch == '\'' {
            // A char literal, or a lifetime.
            if c.get(i + 1) == Some(&'\\') {
                let mut j = i + 2;
                while j < c.len() && c[j] != '\'' {
                    j += 1;
                }
                i = j + 1;
            } else if c.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if ch.is_alphanumeric() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(c[start..i].iter().collect()));
        } else {
            out.push(Tok::Punct(ch));
            i += 1;
        }
    }
    out
}

fn is_raw_start(c: &[char], i: usize) -> bool {
    // An identifier can't be right before it.
    if i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_') {
        return false;
    }
    match (c[i], c.get(i + 1), c.get(i + 2)) {
        ('r', Some('"'), _) | ('r', Some('#'), _) => {
            let rest = &c[i + 1..];
            let h = rest.iter().take_while(|&&x| x == '#').count();
            rest.get(h) == Some(&'"')
        }
        ('b', Some('"'), _) => true,
        ('b', Some('r'), Some('"' | '#')) => true,
        _ => false,
    }
}

/// An ordinary string literal from just after its opening quote: its
/// value and the index after the closing quote.
fn string(c: &[char], mut i: usize) -> (String, usize) {
    let mut s = String::new();
    while i < c.len() {
        match c[i] {
            '"' => return (s, i + 1),
            '\\' => {
                i += 1;
                match c.get(i) {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('0') => s.push('\0'),
                    Some('u') => {
                        let mut j = i + 2;
                        let mut hex = String::new();
                        while j < c.len() && c[j] != '}' {
                            hex.push(c[j]);
                            j += 1;
                        }
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            s.push(ch);
                        }
                        i = j;
                    }
                    Some('x') => {
                        let hex: String = c[i + 1..(i + 3).min(c.len())].iter().collect();
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            s.push(ch);
                        }
                        i += 2;
                    }
                    Some('\n') => {
                        while c.get(i + 1).is_some_and(|x| x.is_whitespace()) {
                            i += 1;
                        }
                    }
                    Some(&x) => s.push(x),
                    None => {}
                }
                i += 1;
            }
            x => {
                s.push(x);
                i += 1;
            }
        }
    }
    (s, i)
}

fn arabic(s: &str) -> bool {
    s.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c))
}

/// The tokens without `#[cfg(test)] mod … { … }` blocks.
fn without_tests(t: Vec<Tok>) -> Vec<Tok> {
    let pat = [Tok::Punct('#'), Tok::Punct('['), Tok::Ident("cfg".into()), Tok::Punct('('), Tok::Ident("test".into()), Tok::Punct(')'), Tok::Punct(']')];
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if t[i..].starts_with(&pat) {
            // Further attributes, then `mod name {`.
            let mut j = i + pat.len();
            while t.get(j) == Some(&Tok::Punct('#')) {
                let mut depth = 0;
                j += 1;
                while j < t.len() {
                    match &t[j] {
                        Tok::Punct('[') => depth += 1,
                        Tok::Punct(']') => {
                            depth -= 1;
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
            }
            if t.get(j) == Some(&Tok::Ident("mod".into())) && t.get(j + 2) == Some(&Tok::Punct('{')) {
                let mut depth = 0;
                let mut k = j + 2;
                while k < t.len() {
                    match &t[k] {
                        Tok::Punct('{') => depth += 1,
                        Tok::Punct('}') => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                i = k + 1;
                continue;
            }
        }
        out.push(t[i].clone());
        i += 1;
    }
    out
}

/// Brand names: never translated.
const KEEP: &[&str] = &["InstaFind"];

/// (English, Arabic) of every text in `src/` (tests excluded).
fn rust_sources() -> BTreeMap<String, String> {
    let mut files = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(d).unwrap().filter_map(Result::ok).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    walk(&root().join("src"), &mut files);
    let mut map = BTreeMap::new();
    let mut add = |en: &str, ar: &str| {
        if !en.trim().is_empty() && !KEEP.contains(&en) {
            map.entry(en.to_string()).or_insert_with(|| ar.to_string());
        }
    };
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        if name == "i18n.rs" || name == "i18n_check.rs" {
            continue;
        }
        let t = without_tests(lex(&std::fs::read_to_string(&f).unwrap()));
        // Word lists for matching (app names, units, letters) pair Arabic
        // and English too, but are not interface texts. The settings pages'
        // titles and sections are (`Page::title`, `Page::section`); their
        // keywords are in arrays, which this does not read.
        let rel = f.strip_prefix(root().join("src")).unwrap().to_string_lossy().replace('\\', "/");
        let tables_are_texts = (rel == "launcher/settings_pages.rs" || !["launcher/", "extract/", "search/"].iter().any(|d| rel.starts_with(d)))
            && !matches!(name.as_str(), "franco.rs" | "normalize.rs" | "suggest.rs" | "fuzzy.rs");
        let s = |i: usize| match t.get(i) {
            Some(Tok::Str(s)) => Some(s.as_str()),
            _ => None,
        };
        let is = |i: usize, p: char| t.get(i) == Some(&Tok::Punct(p));
        let id = |i: usize, n: &str| matches!(t.get(i), Some(Tok::Ident(x)) if x == n);
        let mut arrays: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for i in 0..t.len() {
            // tr!("ar", "en") and trf!("ar", "en", …)
            if (id(i, "tr") || id(i, "trf")) && is(i + 1, '!') && is(i + 2, '(') && is(i + 4, ',')
                && let (Some(ar), Some(en)) = (s(i + 3), s(i + 5))
            {
                add(en, ar);
            }
            // trf!(in lang; "ar", "en", …)
            if id(i, "trf") && is(i + 1, '!') && is(i + 2, '(') && id(i + 3, "in") {
                let mut j = i + 4;
                let mut depth = 0i32;
                while j < t.len() && depth >= 0 && !(depth == 0 && is(j, ';')) {
                    match &t[j] {
                        Tok::Punct('(' | '[' | '{') => depth += 1,
                        Tok::Punct(')' | ']' | '}') => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                if is(j + 2, ',')
                    && let (Some(ar), Some(en)) = (s(j + 1), s(j + 3))
                {
                    add(en, ar);
                }
            }
            // .pick("ar", "en")
            if is(i, '.') && id(i + 1, "pick") && is(i + 2, '(') && is(i + 4, ',')
                && let (Some(ar), Some(en)) = (s(i + 3), s(i + 5))
            {
                add(en, ar);
            }
            // ("عربي", "English" …) and ("key", "عربي", "English" …) in tables.
            for at in [i + 1, i + 3] {
                if tables_are_texts
                    && is(i, '(')
                    && (at == i + 1 || (s(i + 1).is_some() && is(i + 2, ',')))
                    && is(at + 1, ',')
                    && (is(at + 3, ')') || is(at + 3, ','))
                    && let (Some(ar), Some(en)) = (s(at), s(at + 2))
                    && arabic(ar)
                    && !arabic(en)
                    && en.chars().any(|c| c.is_ascii_alphabetic())
                {
                    add(en, ar);
                }
            }
            // const NAME: … = [ "…", … ]; or = "…";
            if id(i, "const") && let Some(Tok::Ident(n)) = t.get(i + 1) {
                // Past the type, whose `[&str; 12]` has a `;` of its own.
                let mut j = i + 2;
                let mut in_type = 0i32;
                while j < t.len() && !is(j, '=') && !(in_type == 0 && is(j, ';')) {
                    match &t[j] {
                        Tok::Punct('[' | '(') => in_type += 1,
                        Tok::Punct(']' | ')') => in_type -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                let mut strs = Vec::new();
                let mut depth = 0i32;
                while j < t.len() {
                    match &t[j] {
                        Tok::Punct('[' | '(' | '{') => depth += 1,
                        Tok::Punct(']' | ')' | '}') => depth -= 1,
                        Tok::Punct(';') if depth == 0 => break,
                        // The first text of each tuple only (KINDS: (label, filter)).
                        Tok::Str(x) if !(is(j - 1, ',') && depth >= 2) => strs.push(x.clone()),
                        _ => {}
                    }
                    j += 1;
                }
                arrays.insert(n.clone(), strs);
            }
        }
        for (n, en) in &arrays {
            // NAME_EN beside NAME or NAME_AR.
            if let Some(ar) = n.strip_suffix("_EN").and_then(|base| arrays.get(base).or_else(|| arrays.get(&format!("{base}_AR"))))
                && ar.len() == en.len()
            {
                for (a, e) in ar.iter().zip(en) {
                    add(e, a);
                }
            }
        }
    }
    // The file-type labels: Arabic in search::KINDS, English in i18n.rs.
    for ((ar, _), en) in crate::search::KINDS.iter().zip(super::KIND_LABELS_EN) {
        add(en, ar);
    }
    map
}

// ------------------------------------------------------------------ app.js

/// The `T` table of app.js: `{ "ar": {…}, "en": {…} }`, a function value
/// as `"FN:<its source>"`.
fn js_tables() -> Value {
    let js = std::fs::read_to_string(root().join("src/webui/assets/app.js")).unwrap();
    let start = js.find("const T = ").expect("app.js: const T");
    let c: Vec<char> = js[start..].chars().collect();
    let open = c.iter().position(|&x| x == '{').unwrap();
    let mut p = Js { c: &c, i: open };
    p.value()
}

struct Js<'a> {
    c: &'a [char],
    i: usize,
}

impl Js<'_> {
    fn peek(&self) -> char {
        self.c.get(self.i).copied().unwrap_or('\0')
    }

    fn ws(&mut self) {
        loop {
            while self.peek().is_whitespace() {
                self.i += 1;
            }
            if self.peek() == '/' && self.c.get(self.i + 1) == Some(&'/') {
                while self.i < self.c.len() && self.peek() != '\n' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    fn value(&mut self) -> Value {
        self.ws();
        let start = self.i;
        match self.peek() {
            '{' => {
                self.i += 1;
                let mut m = Map::new();
                loop {
                    self.ws();
                    if self.peek() == '}' {
                        self.i += 1;
                        break;
                    }
                    let key = if matches!(self.peek(), '"' | '\'') {
                        let q = self.peek();
                        self.quoted(q)
                    } else {
                        let s = self.i;
                        while self.peek().is_alphanumeric() || self.peek() == '_' || self.peek() == '$' {
                            self.i += 1;
                        }
                        self.c[s..self.i].iter().collect()
                    };
                    self.ws();
                    assert_eq!(self.peek(), ':', "app.js T: ':' after {key}");
                    self.i += 1;
                    let v = self.value();
                    m.insert(key, v);
                    self.ws();
                    if self.peek() == ',' {
                        self.i += 1;
                    }
                }
                Value::Object(m)
            }
            '[' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.peek() == ']' {
                        self.i += 1;
                        break;
                    }
                    v.push(self.value());
                    self.ws();
                    if self.peek() == ',' {
                        self.i += 1;
                    }
                }
                Value::Array(v)
            }
            q @ ('"' | '\'') => {
                let s = self.quoted(q);
                // A plain text, unless an expression follows (`"a" + b`).
                self.ws();
                if matches!(self.peek(), ',' | '}' | ']') {
                    Value::String(s)
                } else {
                    self.i = start;
                    self.expression()
                }
            }
            _ => self.expression(),
        }
    }

    /// Anything else (a function, a template): its source up to the next
    /// `,` or closing bracket at its own level.
    fn expression(&mut self) -> Value {
        let start = self.i;
        self.skip_expr();
        let src: String = self.c[start..self.i].iter().collect();
        let src = src.trim().to_string();
        if src.starts_with('`') && !src.contains("${") && src.ends_with('`') {
            return Value::String(src[1..src.len() - 1].to_string());
        }
        Value::String(format!("FN:{src}"))
    }

    fn skip_expr(&mut self) {
        let mut depth = 0i32;
        while self.i < self.c.len() {
            match self.peek() {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                ',' if depth == 0 => return,
                q @ ('"' | '\'') => {
                    self.quoted(q);
                    continue;
                }
                '`' => {
                    self.template();
                    continue;
                }
                _ => {}
            }
            self.i += 1;
        }
    }

    fn template(&mut self) {
        self.i += 1;
        while self.i < self.c.len() {
            match self.peek() {
                '`' => {
                    self.i += 1;
                    return;
                }
                '\\' => self.i += 2,
                '$' if self.c.get(self.i + 1) == Some(&'{') => {
                    self.i += 2;
                    self.skip_expr();
                    self.i += 1; // the closing brace
                }
                _ => self.i += 1,
            }
        }
    }

    fn quoted(&mut self, q: char) -> String {
        self.i += 1;
        let mut s = String::new();
        while self.i < self.c.len() {
            let ch = self.peek();
            self.i += 1;
            if ch == q {
                break;
            }
            if ch == '\\' {
                let e = self.peek();
                self.i += 1;
                s.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    x => x,
                });
            } else {
                s.push(ch);
            }
        }
        s
    }
}

/// `{"a": {"b": "x"}, "c": ["y"]}` → `a.b` = x, `c.0` = y.
fn flatten(v: &Value, prefix: &str, out: &mut BTreeMap<String, Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                flatten(x, &if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, out);
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                flatten(x, &format!("{prefix}.{i}"), out);
            }
        }
        x => {
            out.insert(prefix.to_string(), x.clone());
        }
    }
}

// ------------------------------------------------------------------ app.js functions

/// The parameters of an English function's source, `(n, ms) => …`.
fn params(src: &str) -> Option<Vec<String>> {
    let src = src.trim();
    let head = &src[..src.find("=>")?];
    let head = head.trim().trim_start_matches('(').trim_end_matches(')');
    Some(head.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect())
}

/// Checks a translation of an app.js function as app.js reads it
/// (`compile` in app.js): `FN:(params) => expression`, or a template text
/// with `${param}` places. The names used must be the English function's
/// parameters, `num`, or `T.<key>`.
fn check_fn(en: &str, t: &str) -> Result<(), String> {
    let want = params(en.strip_prefix("FN:").unwrap_or(en)).ok_or("English is not a function")?;
    let (names, body, template) = match t.strip_prefix("FN:") {
        Some(f) => {
            let p = params(f).ok_or("no `=>`")?;
            (p, f[f.find("=>").unwrap() + 2..].to_string(), false)
        }
        None => (want.clone(), t.to_string(), true),
    };
    if names != want {
        return Err(format!("parameters {names:?}, English has {want:?}"));
    }
    let c: Vec<char> = body.chars().collect();
    let mut x = Expr { c: &c, i: 0, names: &names };
    if template {
        x.template_body(None)?;
    } else {
        x.expr()?;
        x.ws();
    }
    if x.i != c.len() {
        return Err(format!("unexpected text at {}", x.i));
    }
    Ok(())
}

/// The expression language of app.js's `compile`, checked only.
struct Expr<'a> {
    c: &'a [char],
    i: usize,
    names: &'a [String],
}

impl Expr<'_> {
    fn peek(&self) -> char {
        self.c.get(self.i).copied().unwrap_or('\0')
    }
    fn ws(&mut self) {
        while self.peek().is_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, s: &str) -> bool {
        self.ws();
        let n = s.chars().count();
        if self.c.len() >= self.i + n && self.c[self.i..self.i + n].iter().copied().eq(s.chars()) {
            self.i += n;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<(), String> {
        self.or()?;
        if self.eat("?") {
            self.expr()?;
            if !self.eat(":") {
                return Err("`?` without `:`".into());
            }
            self.expr()?;
        }
        Ok(())
    }
    fn or(&mut self) -> Result<(), String> {
        self.and()?;
        while self.eat("||") {
            self.and()?;
        }
        Ok(())
    }
    fn and(&mut self) -> Result<(), String> {
        self.cmp()?;
        while self.eat("&&") {
            self.cmp()?;
        }
        Ok(())
    }
    fn cmp(&mut self) -> Result<(), String> {
        self.add()?;
        for op in ["===", "!==", "==", "!=", "<=", ">=", "<", ">"] {
            if self.eat(op) {
                return self.add();
            }
        }
        Ok(())
    }
    fn add(&mut self) -> Result<(), String> {
        self.mul()?;
        while self.eat("+") || self.eat("-") {
            self.mul()?;
        }
        Ok(())
    }
    /// `%` for plural rules (`n % 10 === 1`), `*` and `/`.
    fn mul(&mut self) -> Result<(), String> {
        self.unary()?;
        while self.eat("%") || self.eat("*") || self.eat("/") {
            self.unary()?;
        }
        Ok(())
    }
    fn unary(&mut self) -> Result<(), String> {
        if self.eat("!") {
            return self.unary();
        }
        self.primary()
    }
    fn primary(&mut self) -> Result<(), String> {
        self.ws();
        match self.peek() {
            '(' => {
                self.i += 1;
                self.expr()?;
                if !self.eat(")") {
                    return Err("`(` without `)`".into());
                }
            }
            q @ ('"' | '\'') => {
                self.i += 1;
                while self.i < self.c.len() && self.peek() != q {
                    if self.peek() == '\\' {
                        self.i += 1;
                    }
                    self.i += 1;
                }
                if self.peek() != q {
                    return Err("unclosed string".into());
                }
                self.i += 1;
            }
            '`' => {
                self.i += 1;
                self.template_body(Some('`'))?;
            }
            // A list: [a, b, …] (helpKeys gives rows of [key, text]).
            '[' => {
                self.i += 1;
                if !self.eat("]") {
                    loop {
                        self.expr()?;
                        if self.eat("]") {
                            break;
                        }
                        if !self.eat(",") {
                            return Err("bad list".into());
                        }
                        if self.eat("]") {
                            break;
                        }
                    }
                }
            }
            d if d.is_ascii_digit() => {
                while self.peek().is_ascii_digit() || self.peek() == '.' {
                    self.i += 1;
                }
            }
            a if a.is_alphabetic() || a == '_' => {
                let s = self.i;
                while self.peek().is_alphanumeric() || self.peek() == '_' {
                    self.i += 1;
                }
                let name: String = self.c[s..self.i].iter().collect();
                if name == "T" {
                    if !self.eat(".") {
                        return Err("`T` without `.key`".into());
                    }
                    self.ws();
                    while self.peek().is_alphanumeric() || self.peek() == '_' {
                        self.i += 1;
                    }
                } else if name != "num" && !self.names.contains(&name) {
                    return Err(format!("unknown name `{name}`"));
                }
                if self.eat("(") && !self.eat(")") {
                    loop {
                        self.expr()?;
                        if self.eat(")") {
                            break;
                        }
                        if !self.eat(",") {
                            return Err("bad call".into());
                        }
                    }
                }
            }
            x => return Err(format!("unexpected `{x}`")),
        }
        Ok(())
    }
    /// Template text up to `end` (a backtick) or the end.
    fn template_body(&mut self, end: Option<char>) -> Result<(), String> {
        while self.i < self.c.len() {
            match self.peek() {
                '\\' => self.i += 2,
                '$' if self.c.get(self.i + 1) == Some(&'{') => {
                    self.i += 2;
                    self.expr()?;
                    if !self.eat("}") {
                        return Err("`${` without `}`".into());
                    }
                }
                x if Some(x) == end => {
                    self.i += 1;
                    return Ok(());
                }
                _ => self.i += 1,
            }
        }
        match end {
            None => Ok(()),
            Some(_) => Err("unclosed template".into()),
        }
    }
}

// ------------------------------------------------------------------ the checks

fn source() -> Value {
    let js = js_tables();
    let rust: Map<String, Value> = rust_sources().into_iter().map(|(en, ar)| (en, Value::String(ar))).collect();
    serde_json::json!({
        "about": "The English texts of InstaFind to translate (written by `I18N_WRITE_SOURCE=1 cargo test i18n_source`). rust: English text → Arabic original, for i18n/<code>.json's \"rust\" (keys are the exact English text; keep every {placeholder}). js: app.js's T.en (functions as FN:<source>; translate them as FN:(same params) => expression, or as a text with ${param} places), for \"js\"; js_ar: T.ar for reference.",
        "rust": rust,
        "js": js["en"],
        "js_ar": js["ar"],
    })
}

#[test]
fn i18n_source_lists_every_text() {
    let src = source();
    let rust = src["rust"].as_object().unwrap();
    // Some texts every build has.
    for en in ["Could not save the language.", "Documents", "Any size", "Today"] {
        assert!(rust.contains_key(en), "{en} not found");
    }
    assert!(rust.len() > 300, "{}", rust.len());
    assert!(src["js"].as_object().unwrap().len() > 100);
    assert!(src["js"]["nres"].as_str().unwrap().starts_with("FN:(n) =>"));
    assert_eq!(src["js"]["sorts"]["best"], "Best match first");
    // Function sources the checker understands, in both originals.
    for (k, v) in src["js"].as_object().unwrap() {
        if let Some(s) = v.as_str().filter(|s| s.starts_with("FN:")) {
            assert_eq!(check_fn(s, s), Ok(()), "en {k}");
            if let Some(a) = src["js_ar"][k].as_str() {
                assert_eq!(check_fn(s, a), Ok(()), "ar {k}");
            }
        }
    }
    let path = root().join("i18n/source.json");
    let text = serde_json::to_string_pretty(&src).unwrap() + "\n";
    if std::env::var_os("I18N_WRITE_SOURCE").is_some() {
        std::fs::write(&path, &text).unwrap();
    } else if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        eprintln!("i18n/source.json is out of date: run `I18N_WRITE_SOURCE=1 cargo test i18n_source`");
    }
}

#[test]
fn i18n_language_files_are_sound() {
    let src = source();
    let rust = src["rust"].as_object().unwrap();
    let mut js = BTreeMap::new();
    flatten(&src["js"], "", &mut js);
    let mut problems = Vec::new();
    let mut table = Vec::new();
    for (code, text) in super::FILES {
        let f: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                problems.push(format!("{code}: not valid JSON: {e}"));
                continue;
            }
        };
        let row = super::LOCALES.iter().find(|l| l.code == *code).unwrap();
        let meta = &f["meta"];
        for k in ["name_native", "name_en", "rtl", "script", "status"] {
            if meta.get(k).is_none() {
                problems.push(format!("{code}: meta.{k} missing"));
            }
        }
        if meta["rtl"].as_bool() != Some(row.rtl) {
            problems.push(format!("{code}: meta.rtl is {}, LOCALES says {}", meta["rtl"], row.rtl));
        }
        if meta["script"].as_str().is_some_and(|s| !s.eq_ignore_ascii_case(row.script)) {
            problems.push(format!("{code}: meta.script is {}, LOCALES says {}", meta["script"], row.script));
        }
        let (mut done_r, mut stale) = (0, 0);
        for (en, t) in f["rust"].as_object().into_iter().flatten() {
            let Some(t) = t.as_str().filter(|t| !t.trim().is_empty()) else { continue };
            if !rust.contains_key(en) {
                stale += 1;
                continue;
            }
            done_r += 1;
            // Only trf! templates are formatted; a tr! text with braces is plain.
            if super::parse(en).is_some_and(|p| p.iter().any(|x| matches!(x, super::Part::Place(..)))) && !super::same_placeholders(en, t) {
                problems.push(format!("{code}: rust {en:?}: placeholders of {t:?} differ"));
            }
        }
        let mut theirs = BTreeMap::new();
        flatten(f.get("js").unwrap_or(&Value::Null), "", &mut theirs);
        let mut done_j = 0;
        for (k, t) in &theirs {
            let Some(en) = js.get(k) else {
                stale += 1;
                continue;
            };
            let Some(t) = t.as_str().filter(|t| !t.trim().is_empty()) else { continue };
            done_j += 1;
            if let Some(en) = en.as_str().filter(|s| s.starts_with("FN:"))
                && let Err(e) = check_fn(en, t)
            {
                problems.push(format!("{code}: js {k}: {e}: {t:?}"));
            }
        }
        table.push(format!(
            "{code:8} rust {:5.1}% ({done_r}/{})  js {:5.1}% ({done_j}/{})  not in the source: {stale}",
            100.0 * done_r as f64 / rust.len() as f64,
            rust.len(),
            100.0 * done_j as f64 / js.len() as f64,
            js.len()
        ));
    }
    eprintln!("translations (missing ones read English):\n{}", table.join("\n"));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The quoted word of an example's description («école», “café”, „Straße“, 「カメラ」).
fn quoted(desc: &str) -> Option<&str> {
    let open = desc.find(['«', '“', '„', '「'])?;
    let rest = &desc[open + desc[open..].chars().next()?.len_utf8()..];
    let end = rest.find(['»', '”', '“', '」'])?;
    Some(rest[..end].trim_matches(|c: char| c.is_whitespace() || c == '\u{a0}'))
}

/// "Try these" in a Latin, Cyrillic or Japanese interface shows `tipFold`:
/// a word typed plainly that finds it written with accents, ё or katakana.
/// It must be true: the search's own folding (normalize.rs) makes the two
/// the same, in English, Arabic and every language file that has one.
#[test]
fn i18n_fold_examples_are_true() {
    let js = js_tables();
    let mut tips = vec![("en".to_string(), js["en"]["tipFold"].clone()), ("ar".to_string(), js["ar"]["tipFold"].clone())];
    for (code, text) in super::FILES {
        let f: Value = serde_json::from_str(text).unwrap();
        if let Some(t) = f["js"].get("tipFold") {
            tips.push((code.to_string(), t.clone()));
        }
    }
    assert!(tips.len() > 10, "{}", tips.len());
    for (code, t) in tips {
        let (Some(typed), Some(desc)) = (t[0].as_str(), t[1].as_str()) else { panic!("{code}: tipFold is not [typed, description]") };
        let found = quoted(desc).unwrap_or_else(|| panic!("{code}: no quoted word in {desc:?}"));
        assert_ne!(typed, found, "{code}");
        assert_eq!(crate::normalize::normalize(typed), crate::normalize::normalize(found), "{code}: {typed:?} does not find {found:?}");
    }
}

#[test]
fn i18n_checker_reads_app_js_functions() {
    let en = "FN:(n) => (n === 0 ? \"No results\" : `${num(n)} results`)";
    for ok in [
        "FN:(n) => (n === 0 ? \"Aucun résultat\" : `${num(n)} résultats`)",
        "${num(n)} résultats",
        "FN:(n) => `${n} — ${T.nres(n)}`",
        "FN:(n) => `${num(n)} ${n % 10 === 1 && n % 100 !== 11 ? \"результат\" : \"результатов\"}`",
        "Résultats : ${n}",
    ] {
        assert_eq!(check_fn(en, ok), Ok(()), "{ok}");
    }
    for bad in ["FN:(m) => `${m}`", "${x} résultats", "FN:(n) => alert(n)", "FN:(n) => `${n}", "${n"] {
        assert!(check_fn(en, bad).is_err(), "{bad}");
    }
}

#[test]
fn i18n_lexer_reads_literals() {
    let t = lex(r##"tr!("أ", "a \"b\" \
        c"); let x = r#"raw "q""#; // tr!("no", "no")
        'x'; '\n'; fn f<'a>() {}"##);
    assert!(t.contains(&Tok::Str("a \"b\" c".into())));
    assert!(t.contains(&Tok::Str("raw \"q\"".into())));
    assert!(!t.contains(&Tok::Str("no".into())));
}
