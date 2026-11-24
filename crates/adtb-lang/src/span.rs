//! Define source spans and diagnostics.

use std::fmt;
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span {
            start: start as u32,
            end: end as u32,
        }
    }

    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagKind {
    Invalid,
    Denied,
    /// A body expands past this build's limits; the qualified name of the declaration it belongs
    /// to, `None` for ad hoc text or a registered body.
    PastLimits {
        object: Option<String>,
    },
    /// A guarantee of the checker or the normaliser does not hold.
    Internal,
    /// A lookup reached a name a refused declaration left poisoned; nothing reports it.
    Poisoned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub span: Span,
    pub message: String,
    pub kind: DiagKind,
}

impl Diag {
    pub fn new(span: Span, message: impl Into<String>) -> Diag {
        Diag {
            span,
            message: message.into(),
            kind: DiagKind::Invalid,
        }
    }

    pub(crate) fn internal(span: Span, message: impl Into<String>) -> Diag {
        Diag {
            span,
            message: message.into(),
            kind: DiagKind::Internal,
        }
    }

    pub(crate) fn denied(span: Span, message: impl Into<String>) -> Diag {
        Diag {
            span,
            message: message.into(),
            kind: DiagKind::Denied,
        }
    }

    pub(crate) fn poisoned(span: Span) -> Diag {
        Diag {
            span,
            message: String::new(),
            kind: DiagKind::Poisoned,
        }
    }

    /// Returns the 1-based line and column of the start of the span.
    pub fn line_col(&self, src: &str) -> (usize, usize) {
        let upto = &src[..(self.span.start as usize).min(src.len())];
        let line = upto.matches('\n').count() + 1;
        let col = upto.rfind('\n').map_or(upto.len(), |i| upto.len() - i - 1) + 1;
        (line, col)
    }

    pub fn render(&self, src: &str) -> String {
        let (line, col) = self.line_col(src);
        format!("{line}:{col}: {}", self.message)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

impl FileId {
    pub const PRELUDE: FileId = FileId(u32::MAX);
    pub const ENGINE: FileId = FileId(u32::MAX - 1);
    /// The file of the header a module with no declarations is built from.
    pub const EMPTY_MODULE: FileId = FileId(u32::MAX - 2);

    /// Returns whether the file holds the engine's own declarations, the prelude's included.
    pub fn is_engine(self) -> bool {
        self == FileId::PRELUDE || self == FileId::ENGINE
    }
}

/// A diagnostic in one of several source files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiag {
    pub file: FileId,
    pub diag: Diag,
}
