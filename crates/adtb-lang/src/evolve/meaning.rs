use super::SourceDecl;
use crate::lex::{Kw, Tok, lex};

/// A token of a declaration's text: its kind and its text.
type Word = (Tok, String);

/// Returns the tokens of `text`, the end left out.
fn words(text: &str) -> Vec<Word> {
    lex(text)
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| (t.tok, text[t.span.range()].to_string()))
        .collect()
}

/// Returns `words` as text, one space between tokens.
fn joined(words: &[Word]) -> String {
    words
        .iter()
        .map(|(_, w)| w.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns declarations `ds` by module and text, in order of both, each text as its tokens with
/// a relation's clauses in sorted order, the arms of a `match` over constructors in sorted
/// order, and a field's `on delete restrict` dropped.
pub(crate) fn decls(ds: &[SourceDecl]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = ds
        .iter()
        .map(|d| (d.module.clone(), normalized(&words(&d.text))))
        .collect();
    out.sort();
    out
}

fn normalized(words: &[Word]) -> String {
    match words.first() {
        Some((Tok::Kw(Kw::Type), _)) => joined(&without_restrict(words)),
        Some((Tok::Kw(Kw::Relation), _)) => {
            let (head, mut clauses) = split_clauses(words);
            clauses.sort();
            [head]
                .into_iter()
                .chain(clauses)
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => arms_sorted(words),
    }
}

/// Returns `words` without each `on delete restrict`.
fn without_restrict(words: &[Word]) -> Vec<Word> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let three: Vec<&str> = words[at..]
            .iter()
            .take(3)
            .map(|(_, w)| w.as_str())
            .collect();
        if three == ["on", "delete", "restrict"] {
            at += 3;
            continue;
        }
        out.push(words[at].clone());
        at += 1;
    }
    out
}

/// Returns `words` as text with the arms of its first `match` sorted, unless an arm holds a
/// `match` of its own outside brackets, whose arms the first `match`'s would take in, or an arm's
/// pattern is not a constructor's, so that the arms match first to last.
fn arms_sorted(words: &[Word]) -> String {
    let mut depth = 0i32;
    let mut cuts = Vec::new();
    let mut start = None;
    for (at, (tok, _)) in words.iter().enumerate() {
        match tok {
            Tok::LParen | Tok::LBrace | Tok::LBracket => depth += 1,
            Tok::RParen | Tok::RBrace | Tok::RBracket => depth -= 1,
            Tok::Kw(Kw::Match) if depth == 0 && start.is_some() => return joined(words),
            Tok::Kw(Kw::Match) if depth == 0 => start = Some(at),
            Tok::Bar if depth == 0 && start.is_some() => cuts.push(at),
            _ => {}
        }
    }
    let Some(&first) = cuts.first() else {
        return joined(words);
    };
    let ends = cuts.iter().skip(1).copied().chain([words.len()]);
    let arms: Vec<&[Word]> = cuts.iter().zip(ends).map(|(&a, b)| &words[a..b]).collect();
    if !arms.iter().all(|a| ctor_pattern(&a[1..])) {
        return joined(words);
    }
    let mut arms: Vec<String> = arms.into_iter().map(joined).collect();
    arms.sort();
    [joined(&words[..first])]
        .into_iter()
        .chain(arms)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns whether arm `words`, after its `|`, starts with a constructor's pattern: `C` or
/// `m::C`.
fn ctor_pattern(words: &[Word]) -> bool {
    match words {
        [(Tok::Upper, _), ..] => true,
        [(Tok::Lower, _), (_, sep), (Tok::Upper, _), ..] => sep == "::",
        _ => false,
    }
}

/// Returns a relation declaration's text up to its first clause, and each clause, as tokens.
#[cfg(test)]
pub(crate) fn relation_clauses(text: &str) -> (String, Vec<String>) {
    split_clauses(&words(text))
}

fn split_clauses(words: &[Word]) -> (String, Vec<String>) {
    let starts = |at: usize| match words[at].1.as_str() {
        "unique" | "interval" | "default" | "computed" | "key" | "volatile" => true,
        "index" => at == 0 || words[at - 1].1 != "interval",
        _ => false,
    };
    let first = (0..words.len())
        .find(|&at| starts(at))
        .unwrap_or(words.len());
    let mut out = Vec::new();
    let mut at = first;
    while at < words.len() {
        let mut end = at + 1;
        while end < words.len() && !starts(end) {
            end += 1;
        }
        out.push(joined(&words[at..end]));
        at = end;
    }
    (joined(&words[..first]), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> String {
        let d = SourceDecl {
            module: "m".into(),
            text: text.into(),
        };
        decls(&[d]).remove(0).1
    }

    /// Two declarations mean the same exactly when their normal forms are equal: spacing and
    /// the order of arms over constructors do not count; a string literal's words and the order
    /// of first-match arms do.
    #[test]
    fn a_declaration_means_the_same_across_spacing_and_constructor_arm_order_only() {
        assert_eq!(
            one("fn f (x: T) -> I64 = match x | A a -> 1 | B b -> 2"),
            one("fn f(x:T)->I64 = match x\n  | B b -> 2\n  | A a -> 1"),
        );
        assert_ne!(
            one("fn f (x: I64) -> I64 = match x | 1 -> 2 | _ -> 3"),
            one("fn f (x: I64) -> I64 = match x | _ -> 3 | 1 -> 2"),
        );
        assert_ne!(
            one("relation r : R key id default x = \"p unique q\""),
            one("relation r : R key id default x = \"p  unique q\""),
        );
        assert_eq!(
            one("relation r : R key id unique (a) unique (b)"),
            one("relation r : R key id unique (b) unique (a)"),
        );
    }
}
