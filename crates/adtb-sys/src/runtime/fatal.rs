use std::io::Write;

use super::IoError;

/// Prefix the diagnostic of a fatal storage I/O failure.
pub const FATAL_IO: &str = "fatal I/O error";

/// Carry the cause of a storage I/O failure that must stop the process.
#[derive(Debug)]
pub struct FatalIo {
    context: String,
    cause: IoError,
}

impl FatalIo {
    /// Returns the diagnostic context of the failed operation.
    pub fn context(&self) -> &str {
        &self.context
    }

    /// Returns the I/O error that stopped the process.
    pub fn cause(&self) -> IoError {
        self.cause
    }
}

impl std::fmt::Display for FatalIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{FATAL_IO}: {}: {:?}", self.context, self.cause)
    }
}

/// Stop with the fatal I/O payload after a best-effort diagnostic.
#[track_caller]
pub fn fatal(context: impl Into<String>, cause: IoError) -> ! {
    let failure = FatalIo {
        context: context.into(),
        cause,
    };
    drop(writeln!(std::io::stderr().lock(), "{failure}"));
    std::panic::panic_any(failure)
}
