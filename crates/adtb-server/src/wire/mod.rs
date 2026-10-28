pub mod frame;
pub mod primitives;
mod reassembly;

pub use reassembly::{Message, MessageLimits, Reassembler};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Incomplete {
        needed: usize,
    },
    Overflow,
    NonCanonicalVarint,
    InvalidUtf8,
    NonNfcText,
    InvalidOption(u8),
    UnknownOption(u64),
    OptionOrder {
        previous: u64,
        actual: u64,
    },
    Limit {
        resource: &'static str,
        actual: u64,
        max: u64,
    },
    InvalidFlags(u8),
    Reserved(u16),
    UnknownKind(u8),
    WrongDirection(u8),
    WrongStream {
        kind: u8,
        stream: u32,
    },
    KindChanged {
        stream: u32,
        expected: u8,
        actual: u8,
    },
    CompressedFrame,
    LengthMismatch {
        declared: u32,
        actual: usize,
    },
    DecoderFailed,
    TrailingBytes(usize),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(super) fn limit(resource: &'static str, actual: usize, max: usize) -> Result<()> {
    if actual > max {
        Err(Error::Limit {
            resource,
            actual: actual as u64,
            max: max as u64,
        })
    } else {
        Ok(())
    }
}
