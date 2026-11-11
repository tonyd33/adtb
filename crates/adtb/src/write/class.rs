use crate::exec::{self, Class};
use crate::storage::PageError;

use super::{Broken, By, Error};

impl Error {
    pub fn class(&self) -> Class {
        match self {
            Error::FileChanged(_) => Class::Retry,
            Error::Rejected(_)
            | Error::Denied(_)
            | Error::SequenceExhausted
            | Error::Refused(_)
            | Error::Closed => Class::Reject,
            Error::Corrupt(_) => Class::Damage,
            Error::Transactions(_) => Class::Transactions,
            Error::Quarantined { by, .. } => by.class(),
            Error::Internal(b) => b.class(),
            Error::Blocked { .. } => Class::Suspended,
            Error::Exec(e) => e.class(),
        }
    }

    /// Returns the damaged page the failure met, if the failure is one not yet mapped.
    pub fn page(&self) -> Option<PageError> {
        match self {
            Error::Exec(exec::Error::Page(p)) => Some(*p),
            _ => None,
        }
    }

    /// Returns the owner of the index whose entry names an unused slot, if the failure is one
    /// not yet mapped.
    pub fn orphan(&self) -> Option<u32> {
        match self {
            Error::Exec(exec::Error::OrphanEntry(i)) => Some(*i),
            _ => None,
        }
    }
}

impl By {
    fn class(self) -> Class {
        match self {
            By::Met => Class::Damage,
            By::Found => Class::Found,
            By::Admitted => Class::Refused,
            By::Transactions => Class::Transactions,
        }
    }
}

impl Broken {
    fn class(&self) -> Class {
        match self {
            Broken::BoundedPlan(_)
            | Broken::StatementPlan(_)
            | Broken::DuplicateKey(_)
            | Broken::DenialNotSet(_)
            | Broken::UniqueUnindexed { .. }
            | Broken::KeyNotFields(_)
            | Broken::ReadOnly
            | Broken::ScanPathNotText
            | Broken::RowsNotSet(_)
            | Broken::FailNotText
            | Broken::NoArm(_)
            | Broken::Unreachable
            | Broken::NotAWrite(_)
            | Broken::ReplaceNotLam
            | Broken::RejectsWraps(_)
            | Broken::NotABoundWrite(_)
            | Broken::ScanUnread
            | Broken::RejectedRowNotRecord
            | Broken::NoUnique(_)
            | Broken::ReferenceUnindexed { .. }
            | Broken::TopNotFor
            | Broken::RowsNotSetTyped(_)
            | Broken::InputNotSet(_)
            | Broken::TopBinder(_)
            | Broken::PriorNotSet(_)
            | Broken::Unprintable(_)
            | Broken::Undeclared { .. }
            | Broken::NoValueOutcome
            | Broken::NoSettingOutcome
            | Broken::NoPointOutcome
            | Broken::NoTrialOutcome
            | Broken::RefusalUnrecorded(_)
            | Broken::WitnessDenialUnrecorded(_)
            | Broken::CallKeyNotInt
            | Broken::ExportNotSetTyped
            | Broken::ExportNotSet
            | Broken::NoViolation
            | Broken::SettingNotRecord
            | Broken::SettingField(_)
            | Broken::CatalogError(_)
            | Broken::DeploymentRefused => Class::Checker,
        }
    }
}
