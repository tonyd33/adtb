use crate::runtime::IoError;

use super::Lsn;
use super::error::io_text;
use super::pager::PageError;

/// Why the read-in gate is not armed for this open: a read the device failed, or one that
/// succeeded only after the open read pages unchecked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unarmed {
    /// Listing the log directory, or opening or reading the segment at `path`, failed.
    Log { path: String, error: IoError },
    /// The block of the retained log holding this LSN could not be read.
    LogBlock(Lsn),
    /// A page of the page LSN map or the damage list could not be read.
    Page(PageError),
    /// A later attempt read what had failed.
    Late,
}

impl std::fmt::Display for Unarmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unarmed::Log { path, error } => {
                write!(f, "the log at `{path}` cannot be read: {}", io_text(*error))
            }
            Unarmed::LogBlock(lsn) => {
                write!(f, "the log block holding LSN {} cannot be read", lsn.0)
            }
            Unarmed::Page(e) => write!(f, "{e}"),
            Unarmed::Late => f.write_str(
                "this open read pages it could not check, so the database must be reopened",
            ),
        }
    }
}

/// How the end of recovery armed the read-in gate.
#[derive(Debug)]
pub(crate) enum Arming {
    /// Every page has the LSN the log proves.
    Proven,
    /// The gate checks the LSNs the open could read, short of a proof: the map page refused, if
    /// that is why.
    Unproven(Option<PageError>),
    /// A read the gate needs failed: no page read in is checked, and verify checks against what
    /// the open could read.
    Failed(Unarmed),
}
