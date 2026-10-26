use adtb_lang::ast::AdhocItem;
use adtb_lang::lex::{Tok, lex};
use adtb_lang::parse::{Options, parse_adhoc};

/// Whether the lines read so far form a text to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// No token: blank lines and comments only.
    Empty,
    /// The text parses, or fails before its end.
    Whole,
    /// The text parses and ends in a `transact` block, whose statements run on to the end of
    /// the text.
    Block,
    /// The text fails only at its end.
    Unfinished,
}

/// Returns the shape of `text` as the ad hoc parser sees it.
pub fn shape(text: &str) -> Shape {
    if lex(text).0.iter().all(|t| t.tok == Tok::Eof) {
        return Shape::Empty;
    }
    let parsed = parse_adhoc(text, Options::default());
    let end = text.trim_end().len();
    let at_end = |d: &adtb_lang::span::Diag| d.span.end as usize >= end;
    match parsed.value {
        Some(items) if parsed.diags.is_empty() => match items.last() {
            Some(AdhocItem::Transact(_)) => Shape::Block,
            _ => Shape::Whole,
        },
        _ if !parsed.diags.is_empty() && parsed.diags.iter().all(at_end) => Shape::Unfinished,
        _ => Shape::Whole,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_query_is_whole() {
        assert_eq!(Shape::Whole, shape("from u in users select u.name"));
    }

    #[test]
    fn a_query_cut_short_is_unfinished() {
        assert_eq!(Shape::Unfinished, shape("from u in users"));
        assert_eq!(
            Shape::Unfinished,
            shape("from u in users select u.name ==\n")
        );
    }

    #[test]
    fn an_unterminated_text_literal_is_unfinished() {
        assert_eq!(Shape::Unfinished, shape("insert users { name: \"ann"));
    }

    #[test]
    fn an_unclosed_record_is_unfinished() {
        assert_eq!(Shape::Unfinished, shape("insert users {\n  name: \"ann\","));
    }

    #[test]
    fn a_transact_block_is_a_block() {
        assert_eq!(Shape::Block, shape("transact"));
        assert_eq!(
            Shape::Block,
            shape("transact\n  insert users { name: \"ann\" }")
        );
    }

    #[test]
    fn an_error_before_the_end_is_whole() {
        assert_eq!(Shape::Whole, shape("from u in ) users select u"));
    }

    #[test]
    fn a_text_of_no_tokens_is_empty() {
        assert_eq!(Shape::Empty, shape(""));
        assert_eq!(Shape::Empty, shape("-- a comment\n  "));
    }
}
