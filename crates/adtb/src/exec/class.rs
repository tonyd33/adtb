use super::Error;

/// What kind of failure an error is, which decides whether it ends the transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// An operating failure after which the transaction cannot go on; running it again may
    /// succeed.
    Retry,
    /// An operating failure of the statement: it has no effect and the transaction goes on.
    Reject,
    /// A page that fails verification, or a stored structure that does not decode.
    Damage,
    /// A failure of the transactions layer.
    Transactions,
    /// An index entry that names an unused slot.
    Found,
    /// A quarantine mark refused the call.
    Refused,
    /// A guarantee the checker makes about a program does not hold.
    Checker,
    /// An invariant over engine state does not hold.
    Engine,
}

impl Class {
    /// Returns whether a failure of this class ends the transaction: the caller must abort it.
    pub fn ends(self) -> bool {
        match self {
            Class::Retry | Class::Damage | Class::Transactions => true,
            Class::Reject | Class::Found | Class::Refused | Class::Checker | Class::Engine => false,
        }
    }
}

impl Error {
    pub fn class(&self) -> Class {
        match self {
            Error::Serialization(_) => Class::Retry,
            Error::Fault(_)
            | Error::StatementLimit(_)
            | Error::Spill(_)
            | Error::Unsupported(_)
            | Error::NoCaller
            | Error::Rejected(_)
            | Error::Cancelled
            | Error::DeadlineExceeded
            | Error::MissingFunction(_)
            | Error::HistoryUnavailable { .. }
            | Error::WrongVersion { .. } => Class::Reject,
            Error::Page(_) | Error::Corrupt(_) => Class::Damage,
            Error::OrphanEntry(_) => Class::Found,
            Error::OtherVersion(_) | Error::Internal(_) => Class::Checker,
        }
    }
}
