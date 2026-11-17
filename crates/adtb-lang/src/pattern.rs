//! Compile and run `matches` patterns.

use std::fmt;

use regex::{Regex, RegexBuilder};
use regex_syntax::ast::{self, Ast, Flag, FlagsItemKind, GroupKind};
use regex_syntax::hir;

/// The most bytes a pattern's compiled form may take.
pub const SIZE_LIMIT: usize = 10 * (1 << 20);

/// A compiled `matches` pattern.
#[derive(Clone, Debug)]
pub struct Pattern(Regex);

/// Why a text is no `matches` pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The text is no regular expression, for `reason`, at a 1-based `line` and `column`.
    Syntax {
        reason: &'static str,
        line: usize,
        column: usize,
    },
    /// The text sets `flag`, which is none of `i`, `m`, `s` and `U`.
    Flag {
        flag: char,
        line: usize,
        column: usize,
    },
    /// The compiled form exceeds [`SIZE_LIMIT`].
    TooBig,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = |f: &mut fmt::Formatter<'_>, line: usize, column: usize| match line {
            1 => write!(f, ", at column {column}"),
            _ => write!(f, ", at line {line} column {column}"),
        };
        match *self {
            Refusal::Syntax {
                reason,
                line,
                column,
            } => {
                f.write_str(reason)?;
                at(f, line, column)
            }
            Refusal::Flag { flag, line, column } => {
                write!(f, "the flag `{flag}` is none of `i`, `m`, `s` and `U`")?;
                at(f, line, column)
            }
            Refusal::TooBig => write!(
                f,
                "its compiled form exceeds the limit of {SIZE_LIMIT} bytes"
            ),
        }
    }
}

impl Pattern {
    /// Returns `re` compiled, or why it is no pattern.
    pub fn new(re: &str) -> Result<Pattern, Refusal> {
        let tree = ast::parse::Parser::new()
            .parse(re)
            .map_err(|e| syntax(ast_reason(e.kind()), e.span()))?;
        ast::visit(&tree, FlagGate)?;
        hir::translate::Translator::new()
            .translate(re, &tree)
            .map_err(|e| syntax(hir_reason(e.kind()), e.span()))?;
        #[expect(
            clippy::disallowed_methods,
            reason = "the one compile of a pattern; remove when matching no longer uses the regex crate"
        )]
        let built = RegexBuilder::new(re).size_limit(SIZE_LIMIT).build();
        built.map(Pattern).map_err(|e| match e {
            regex::Error::CompiledTooBig(_) => Refusal::TooBig,
            _ => Refusal::Syntax {
                reason: UNSUPPORTED,
                line: 1,
                column: 1,
            },
        })
    }

    /// Returns whether the pattern matches somewhere in `t`.
    pub fn is_match(&self, t: &str) -> bool {
        self.0.is_match(t)
    }
}

const UNSUPPORTED: &str = "a construct matching does not support";

fn syntax(reason: &'static str, span: &ast::Span) -> Refusal {
    Refusal::Syntax {
        reason,
        line: span.start.line,
        column: span.start.column,
    }
}

/// Refuse the first flag outside `i`, `m`, `s` and `U`.
struct FlagGate;

impl ast::Visitor for FlagGate {
    type Output = ();
    type Err = Refusal;

    fn finish(self) -> Result<(), Refusal> {
        Ok(())
    }

    fn visit_pre(&mut self, a: &Ast) -> Result<(), Refusal> {
        let flags = match a {
            Ast::Flags(set) => &set.flags,
            Ast::Group(g) => match &g.kind {
                GroupKind::NonCapturing(flags) => flags,
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };
        for item in &flags.items {
            let FlagsItemKind::Flag(flag) = item.kind else {
                continue;
            };
            let flag = match flag {
                Flag::CaseInsensitive
                | Flag::MultiLine
                | Flag::DotMatchesNewLine
                | Flag::SwapGreed => continue,
                Flag::Unicode => 'u',
                Flag::CRLF => 'R',
                Flag::IgnoreWhitespace => 'x',
            };
            return Err(Refusal::Flag {
                flag,
                line: item.span.start.line,
                column: item.span.start.column,
            });
        }
        Ok(())
    }
}

fn ast_reason(kind: &ast::ErrorKind) -> &'static str {
    use ast::ErrorKind as K;
    match kind {
        K::CaptureLimitExceeded => "too many groups",
        K::ClassEscapeInvalid => "an escape a character class does not take",
        K::ClassRangeInvalid => "a character range whose start is after its end",
        K::ClassRangeLiteral => "a character range bound that is not one character",
        K::ClassUnclosed => "a `[` with no `]`",
        K::DecimalEmpty | K::DecimalInvalid | K::RepetitionCountDecimalEmpty => {
            "a repetition count that is not a number"
        }
        K::EscapeHexEmpty | K::EscapeHexInvalid | K::EscapeHexInvalidDigit => {
            "a hexadecimal escape that is not a scalar value"
        }
        K::EscapeUnexpectedEof => "an escape at the end of the pattern",
        K::EscapeUnrecognized => "an unknown escape",
        K::FlagDanglingNegation
        | K::FlagDuplicate { .. }
        | K::FlagRepeatedNegation { .. }
        | K::FlagUnexpectedEof
        | K::FlagUnrecognized => "a malformed flag group",
        K::GroupNameDuplicate { .. }
        | K::GroupNameEmpty
        | K::GroupNameInvalid
        | K::GroupNameUnexpectedEof => "a malformed group name",
        K::GroupUnclosed => "a `(` with no `)`",
        K::GroupUnopened => "a `)` with no `(`",
        K::NestLimitExceeded(_) => "groups nested too deep",
        K::RepetitionCountInvalid => "a repetition range whose start is after its end",
        K::RepetitionCountUnclosed => "a `{` with no `}`",
        K::RepetitionMissing => "a repetition of nothing",
        K::SpecialWordBoundaryUnclosed
        | K::SpecialWordBoundaryUnrecognized
        | K::SpecialWordOrRepetitionUnexpectedEof => "a malformed word boundary",
        K::UnicodeClassInvalid => "a malformed Unicode class",
        K::UnsupportedBackreference => "a backreference",
        K::UnsupportedLookAround => "a lookaround",
        _ => UNSUPPORTED,
    }
}

fn hir_reason(kind: &hir::ErrorKind) -> &'static str {
    match kind {
        hir::ErrorKind::UnicodePropertyNotFound | hir::ErrorKind::UnicodePropertyValueNotFound => {
            "an unknown Unicode property"
        }
        _ => UNSUPPORTED,
    }
}
