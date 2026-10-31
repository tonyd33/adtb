use crate::span::{Diag, Span};
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kw {
    And,
    Bundle,
    By,
    Check,
    Constraint,
    Delete,
    Else,
    Exists,
    Fail,
    False,
    Fix,
    Fn,
    For,
    From,
    Group,
    If,
    Import,
    In,
    Insert,
    Into,
    Key,
    Let,
    Match,
    Module,
    Not,
    Or,
    Order,
    Ordered,
    Query,
    Relation,
    Return,
    Schema,
    Select,
    Set,
    Skip,
    Temp,
    Then,
    Top,
    Transact,
    True,
    Try,
    Txn,
    Type,
    Union,
    Update,
    Where,
    With,
}

macro_rules! keywords {
    ($($word:literal => $kw:ident,)*) => {
        impl Kw {
            pub const WORDS: &[(&str, Kw)] = &[$(($word, Kw::$kw)),*];

            pub fn from_word(s: &str) -> Option<Kw> {
                Some(match s {
                    $($word => Kw::$kw,)*
                    _ => return None,
                })
            }
        }
    };
}

keywords! {
    "and" => And,
    "bundle" => Bundle,
    "by" => By,
    "check" => Check,
    "constraint" => Constraint,
    "delete" => Delete,
    "else" => Else,
    "exists" => Exists,
    "fail" => Fail,
    "false" => False,
    "fix" => Fix,
    "fn" => Fn,
    "for" => For,
    "from" => From,
    "group" => Group,
    "if" => If,
    "import" => Import,
    "in" => In,
    "insert" => Insert,
    "into" => Into,
    "key" => Key,
    "let" => Let,
    "match" => Match,
    "module" => Module,
    "not" => Not,
    "or" => Or,
    "order" => Order,
    "ordered" => Ordered,
    "query" => Query,
    "relation" => Relation,
    "return" => Return,
    "schema" => Schema,
    "select" => Select,
    "set" => Set,
    "skip" => Skip,
    "temp" => Temp,
    "then" => Then,
    "top" => Top,
    "transact" => Transact,
    "true" => True,
    "try" => Try,
    "txn" => Txn,
    "type" => Type,
    "union" => Union,
    "update" => Update,
    "where" => Where,
    "with" => With,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Lower,
    Upper,
    Kw(Kw),
    Int,
    Decimal,
    Float,
    Text,
    Duration,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    PlusPlus,
    Eq,
    Colon,
    ColonColon,
    Comma,
    Dot,
    DotLt,
    Arrow,
    Bar,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Underscore,
    Error,
    Eof,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// Whitespace or a comment separates this token from the previous one.
    pub spaced: bool,
}

pub const DURATION_UNITS: [&str; 6] = ["us", "ms", "min", "s", "h", "d"];

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Splits source text into tokens, ending with one `Eof` token.
pub fn lex(src: &str) -> (Vec<Token>, Vec<Diag>) {
    let bytes = src.as_bytes();
    let mut toks = Vec::new();
    let mut diags = Vec::new();
    let mut i = 0;
    let mut spaced = true;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
            i += 1;
            spaced = true;
            continue;
        }
        if c == b'-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            spaced = true;
            continue;
        }
        let start = i;
        let tok = if c.is_ascii_lowercase() || c.is_ascii_uppercase() {
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &src[start..i];
            if c.is_ascii_uppercase() {
                Tok::Upper
            } else if let Some(kw) = Kw::from_word(word) {
                Tok::Kw(kw)
            } else {
                Tok::Lower
            }
        } else if c.is_ascii_digit() {
            lex_number(bytes, &mut i)
        } else if c == b'"' {
            lex_text(src, &mut i, &mut diags)
        } else if c == b'_' {
            i += 1;
            if i < bytes.len() && is_ident_char(bytes[i]) {
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                diags.push(Diag::new(
                    Span::new(start, i),
                    "an identifier must start with a letter",
                ));
                Tok::Error
            } else {
                Tok::Underscore
            }
        } else {
            let next = bytes.get(i + 1).copied();
            let (tok, len) = match (c, next) {
                (b'=', Some(b'=')) => (Tok::EqEq, 2),
                (b'!', Some(b'=')) => (Tok::NotEq, 2),
                (b'<', Some(b'=')) => (Tok::Le, 2),
                (b'>', Some(b'=')) => (Tok::Ge, 2),
                (b'+', Some(b'+')) => (Tok::PlusPlus, 2),
                (b':', Some(b':')) => (Tok::ColonColon, 2),
                (b'.', Some(b'<')) => (Tok::DotLt, 2),
                (b'-', Some(b'>')) => (Tok::Arrow, 2),
                (b'<', _) => (Tok::Lt, 1),
                (b'>', _) => (Tok::Gt, 1),
                (b'+', _) => (Tok::Plus, 1),
                (b'-', _) => (Tok::Minus, 1),
                (b'*', _) => (Tok::Star, 1),
                (b'/', _) => (Tok::Slash, 1),
                (b'%', _) => (Tok::Percent, 1),
                (b'=', _) => (Tok::Eq, 1),
                (b':', _) => (Tok::Colon, 1),
                (b',', _) => (Tok::Comma, 1),
                (b'.', _) => (Tok::Dot, 1),
                (b'|', _) => (Tok::Bar, 1),
                (b'(', _) => (Tok::LParen, 1),
                (b')', _) => (Tok::RParen, 1),
                (b'{', _) => (Tok::LBrace, 1),
                (b'}', _) => (Tok::RBrace, 1),
                (b'[', _) => (Tok::LBracket, 1),
                (b']', _) => (Tok::RBracket, 1),
                _ => {
                    let len = src[i..].chars().next().map_or(1, char::len_utf8);
                    diags.push(Diag::new(
                        Span::new(i, i + len),
                        format!("unexpected character {:?}", &src[i..i + len]),
                    ));
                    (Tok::Error, len)
                }
            };
            i += len;
            tok
        };
        toks.push(Token {
            tok,
            span: Span::new(start, i),
            spaced,
        });
        spaced = false;
    }
    toks.push(Token {
        tok: Tok::Eof,
        span: Span::new(bytes.len(), bytes.len()),
        spaced: true,
    });
    (toks, diags)
}

fn lex_number(bytes: &[u8], i: &mut usize) -> Tok {
    let digits = |i: &mut usize| {
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
    };
    digits(i);
    let mut tok = Tok::Int;
    if bytes.get(*i) == Some(&b'.') && bytes.get(*i + 1).is_some_and(u8::is_ascii_digit) {
        *i += 1;
        digits(i);
        tok = Tok::Decimal;
    }
    if matches!(bytes.get(*i), Some(b'e' | b'E')) {
        let mut j = *i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            *i = j;
            digits(i);
            return Tok::Float;
        }
    }
    if tok == Tok::Int {
        let mut j = *i;
        while j < bytes.len() && is_ident_char(bytes[j]) {
            j += 1;
        }
        let word = &bytes[*i..j];
        if DURATION_UNITS.iter().any(|u| u.as_bytes() == word) {
            *i = j;
            return Tok::Duration;
        }
    }
    tok
}

fn lex_text(src: &str, i: &mut usize, diags: &mut Vec<Diag>) -> Tok {
    let bytes = src.as_bytes();
    let start = *i;
    *i += 1;
    loop {
        match bytes.get(*i) {
            None | Some(b'\n') => {
                diags.push(Diag::new(Span::new(start, *i), "unterminated text literal"));
                return Tok::Error;
            }
            Some(b'"') => {
                *i += 1;
                return Tok::Text;
            }
            Some(b'\\') => {
                let esc_start = *i;
                *i += 1;
                match bytes.get(*i) {
                    Some(b'"' | b'\\' | b'n' | b'r' | b't') => *i += 1,
                    Some(b'u') if bytes.get(*i + 1) == Some(&b'{') => {
                        *i += 2;
                        let hex_start = *i;
                        while bytes.get(*i).is_some_and(u8::is_ascii_hexdigit) {
                            *i += 1;
                        }
                        let hex = &src[hex_start..*i];
                        if bytes.get(*i) != Some(&b'}') || hex.is_empty() {
                            diags.push(Diag::new(
                                Span::new(esc_start, *i),
                                "malformed \\u{...} escape",
                            ));
                        } else {
                            *i += 1;
                            let valid = u32::from_str_radix(hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .is_some();
                            if !valid {
                                diags.push(Diag::new(
                                    Span::new(esc_start, *i),
                                    "escape names no Unicode scalar value",
                                ));
                            }
                        }
                    }
                    None => {}
                    _ => {
                        *i += src[*i..].chars().next().map_or(1, char::len_utf8);
                        diags.push(Diag::new(Span::new(esc_start, *i), "unknown escape"));
                    }
                }
            }
            Some(_) => *i += src[*i..].chars().next().map_or(1, char::len_utf8),
        }
    }
}

/// Returns the value of a text literal the lexer accepted, in NFC, quotes included in `raw`.
pub fn decode_text(raw: &str) -> String {
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                chars.next();
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some(c) => out.push(c),
            None => {}
        }
    }
    out.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        let (toks, diags) = lex(src);
        assert_eq!(diags, vec![]);
        toks.into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("3 3.25 1e5 2.5E-3 90d 5min 30s 2e"),
            vec![
                Tok::Int,
                Tok::Decimal,
                Tok::Float,
                Tok::Float,
                Tok::Duration,
                Tok::Duration,
                Tok::Duration,
                Tok::Int,
                Tok::Lower,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn dot_after_int_is_not_decimal() {
        assert_eq!(
            kinds("x.1 1.f"),
            vec![
                Tok::Lower,
                Tok::Dot,
                Tok::Int,
                Tok::Int,
                Tok::Dot,
                Tok::Lower,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn comments_and_spacing() {
        let (toks, _) = lex("f(x) -- note\ng (y)");
        let spaced: Vec<bool> = toks.iter().map(|t| t.spaced).collect();
        assert_eq!(
            spaced,
            vec![true, false, false, false, true, true, false, false, true]
        );
    }

    #[test]
    fn operators() {
        assert_eq!(
            kinds("== != <= >= ++ :: .< -> | _"),
            vec![
                Tok::EqEq,
                Tok::NotEq,
                Tok::Le,
                Tok::Ge,
                Tok::PlusPlus,
                Tok::ColonColon,
                Tok::DotLt,
                Tok::Arrow,
                Tok::Bar,
                Tok::Underscore,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn keywords_and_contextual_words() {
        assert_eq!(
            kinds("from add Key key"),
            vec![
                Tok::Kw(Kw::From),
                Tok::Lower,
                Tok::Upper,
                Tok::Kw(Kw::Key),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn text_escapes() {
        let src = r#""a\"b\n\u{e9}""#;
        assert_eq!(kinds(src), vec![Tok::Text, Tok::Eof]);
        assert_eq!(decode_text(src), "a\"b\n\u{e9}");
    }

    #[test]
    fn a_bad_escape_ends_on_a_character_boundary() {
        for src in ["\"a\\", "\"a\\\u{e9}\"", "\"\\\u{1f600}", "\"\\"] {
            let (toks, diags) = lex(src);
            assert!(!diags.is_empty(), "{src:?}");
            for t in toks {
                assert!(src.get(t.span.range()).is_some(), "{src:?}: {t:?}");
            }
        }
    }

    #[test]
    fn text_literals_are_nfc() {
        assert_eq!(decode_text("\"e\u{301}\""), "\u{e9}");
        assert_eq!(decode_text("\"e\\u{301}\""), "\u{e9}");
        assert_eq!(decode_text("\"\u{e9}\""), "\u{e9}");
        assert_eq!(decode_text("\"\u{212b}\""), "\u{c5}");
        assert_eq!(decode_text("\"\u{1e0b}\u{323}\""), "\u{1e0d}\u{307}");
    }

    #[test]
    fn identifiers_are_ascii() {
        let (_, diags) = lex("caf\u{e9}");
        assert_eq!(diags.len(), 1);
        let (_, diags) = lex("cafe\u{301}");
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn bad_escapes_are_refused() {
        let (_, diags) = lex(r#""\u{d800}" "\q" "open"#);
        assert_eq!(diags.len(), 3);
    }

    #[test]
    fn leading_underscore_identifier_is_refused() {
        let (_, diags) = lex("_x");
        assert_eq!(diags.len(), 1);
    }
}
