//! `trf!` for InstaFind (`src/i18n.rs`): `format!` with the Arabic or the
//! English template, or, in any other interface language, with the
//! translation of the English template found at run time.
//!
//! A translated template is only known at run time, so its placeholders
//! can't go through `format!`. Instead each placeholder of the English
//! template is formatted here on its own, with its own format spec
//! (`format!("{n}")`, `format!("{:.1}", arg)`…), and `crate::i18n::fill`
//! puts those texts in the translation's placeholders. The arguments are
//! evaluated once, whichever template is used.
//!
//! Called by `trf!` in `src/i18n.rs` as
//! `trf_impl!(("ar") ("en") (arg0) (arg1) …)`: each part in its own
//! parentheses, so commas inside an argument need no parsing here.
//!
//! No dependencies (no syn/quote): the code is written as text with marker
//! identifiers, parsed, and the markers are replaced by the caller's own
//! tokens, so the templates keep their spans and `{name}` still captures
//! the caller's variable.

use proc_macro::{Delimiter, Group, Literal, TokenStream, TokenTree};

#[proc_macro]
pub fn trf_impl(input: TokenStream) -> TokenStream {
    let parts: Vec<TokenStream> = input
        .into_iter()
        .filter_map(|t| match t {
            TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis => Some(g.stream()),
            _ => None,
        })
        .collect();
    if parts.len() < 2 {
        return error("trf!: expected (\"ar\") (\"en\") (args)…");
    }
    let (Some(ar), Some(en)) = (literal(&parts[0]), literal(&parts[1])) else {
        return error("trf!: the two templates must be string literals");
    };
    let args = &parts[2..];
    let places = literal_text(&en.to_string()).and_then(|t| placeholders(&t));

    // The arguments, each evaluated once and borrowed (`&T` formats as `T`).
    let n = args.len();
    let binds: String = (0..n).map(|i| format!("__trf_a{i},")).collect();
    let refs: String = (0..n).map(|i| format!("&(__TRF_ARG_{i}__),")).collect();
    let plain = format!(
        "if crate::i18n::is_en() {{ ::std::format!(__TRF_EN__, {binds}) }} else {{ ::std::format!(__TRF_AR__, {binds}) }}"
    );
    let body = match places {
        // A template this file can't take apart ({:1$}, {:.*}…): never translated.
        None => plain,
        Some(places) => {
            let values: String = places
                .iter()
                .enumerate()
                .map(|(k, p)| match p.arg {
                    Arg::Pos(i) if i < n => format!("::std::format!(__TRF_LIT_{k}__, __trf_a{i}),"),
                    Arg::Pos(_) => "::std::string::String::new(),".to_string(),
                    Arg::Named => format!("::std::format!(__TRF_LIT_{k}__),"),
                })
                .collect();
            format!(
                "if let ::std::option::Option::Some(__trf_t) = crate::i18n::template(__TRF_EN__) {{ \
                 crate::i18n::fill(__trf_t, __TRF_EN__, &[{values}]) }} else {{ {plain} }}"
            )
        }
    };
    let code = format!("match ({refs}) {{ ({binds}) => {{ {body} }} }}");
    let Ok(stream) = code.parse::<TokenStream>() else { return error("trf!: internal error") };

    // The per-placeholder templates, spanned like the English one (so a
    // `{name}` resolves where the caller wrote it).
    let lits: Vec<Literal> = literal_text(&en.to_string())
        .and_then(|t| placeholders(&t))
        .unwrap_or_default()
        .iter()
        .map(|p| {
            let mut l = Literal::string(&p.text);
            l.set_span(en.span());
            l
        })
        .collect();
    replace(stream, &|name| {
        Some(match name {
            "__TRF_EN__" => TokenTree::Literal(en.clone()).into(),
            "__TRF_AR__" => TokenTree::Literal(ar.clone()).into(),
            _ => {
                if let Some(i) = name.strip_prefix("__TRF_ARG_").and_then(|s| s.strip_suffix("__")) {
                    args.get(i.parse::<usize>().ok()?)?.clone()
                } else if let Some(k) = name.strip_prefix("__TRF_LIT_").and_then(|s| s.strip_suffix("__")) {
                    TokenTree::Literal(lits.get(k.parse::<usize>().ok()?)?.clone()).into()
                } else {
                    return None;
                }
            }
        })
    })
}

fn error(msg: &str) -> TokenStream {
    format!("::std::compile_error!({msg:?})").parse().unwrap()
}

/// The string literal of a part, looking through the invisible groups a
/// `$x:literal` fragment may come wrapped in.
fn literal(ts: &TokenStream) -> Option<Literal> {
    let mut it = ts.clone().into_iter();
    let first = it.next()?;
    if it.next().is_some() {
        return None;
    }
    match first {
        TokenTree::Literal(l) => {
            let s = l.to_string();
            (s.starts_with('"') || s.starts_with('r')).then_some(l)
        }
        TokenTree::Group(g) if g.delimiter() == Delimiter::None => literal(&g.stream()),
        _ => None,
    }
}

/// Replaces every marker identifier (anywhere, groups included).
fn replace(ts: TokenStream, f: &dyn Fn(&str) -> Option<TokenStream>) -> TokenStream {
    let mut out = TokenStream::new();
    for t in ts {
        match t {
            TokenTree::Ident(ref i) if i.to_string().starts_with("__TRF_") => match f(&i.to_string()) {
                Some(s) => out.extend(s),
                None => out.extend(error("trf!: internal marker")),
            },
            TokenTree::Group(g) => {
                let mut n = Group::new(g.delimiter(), replace(g.stream(), f));
                n.set_span(g.span());
                out.extend(std::iter::once(TokenTree::Group(n)));
            }
            t => out.extend(std::iter::once(t)),
        }
    }
    out
}

/// The text of a string literal as written in the source (`"…"`, `r"…"`,
/// `r#"…"#`), with escapes resolved enough for the braces to be right:
/// `\u{…}` and the others become one character each.
fn literal_text(src: &str) -> Option<String> {
    if let Some(raw) = src.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let inner = raw.get(hashes + 1..raw.len().checked_sub(hashes + 1)?)?;
        return Some(inner.to_string());
    }
    let inner = src.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'u' => {
                // \u{XXXX}
                let mut hex = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    if c != '{' {
                        hex.push(c);
                    }
                }
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            'x' => {
                let hex: String = chars.by_ref().take(2).collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\n' => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            c => out.push(c),
        }
    }
    Some(out)
}

enum Arg {
    /// `{}` (the next one) or `{2}`.
    Pos(usize),
    /// `{name}`: captured from the caller's scope.
    Named,
}

struct Place {
    arg: Arg,
    /// The placeholder alone as a template: `{}` → `{}`, `{1:.2}` → `{:.2}`,
    /// `{name:?}` → `{name:?}`.
    text: String,
}

/// The placeholders of a template, in order; `None` for one this macro
/// does not take apart (widths or precisions from arguments).
fn placeholders(t: &str) -> Option<Vec<Place>> {
    let mut out = Vec::new();
    let mut next = 0;
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
            }
            '{' => {
                let mut inner = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        c => inner.push(c),
                    }
                }
                let (name, spec) = match inner.split_once(':') {
                    Some((n, s)) => (n.trim(), Some(s)),
                    None => (inner.trim(), None),
                };
                if spec.is_some_and(|s| s.contains('$') || s.contains('*')) {
                    return None;
                }
                let spec = spec.map(|s| format!(":{s}")).unwrap_or_default();
                let place = if name.is_empty() {
                    next += 1;
                    Place { arg: Arg::Pos(next - 1), text: format!("{{{spec}}}") }
                } else if let Ok(i) = name.parse::<usize>() {
                    Place { arg: Arg::Pos(i), text: format!("{{{spec}}}") }
                } else {
                    Place { arg: Arg::Named, text: format!("{{{name}{spec}}}") }
                };
                out.push(place);
            }
            _ => {}
        }
    }
    Some(out)
}
