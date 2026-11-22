//! The privileges and capabilities a row of `catalog::grants` names.

/// The privilege a row of `catalog::grants` gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Privilege {
    Read,
    Insert,
    Update,
    Delete,
    Retag,
    Call,
    Capability(Capability),
}

impl Privilege {
    /// Returns the privilege a row names `name`.
    pub fn named(name: &str) -> Option<Privilege> {
        [
            Privilege::Read,
            Privilege::Insert,
            Privilege::Update,
            Privilege::Delete,
            Privilege::Retag,
            Privilege::Call,
        ]
        .into_iter()
        .find(|p| p.name() == name)
        .or_else(|| Capability::named(name).map(Privilege::Capability))
    }

    pub fn name(self) -> &'static str {
        match self {
            Privilege::Read => "read",
            Privilege::Insert => "insert",
            Privilege::Update => "update",
            Privilege::Delete => "delete",
            Privilege::Retag => "retag",
            Privilege::Call => "call",
            Privilege::Capability(c) => c.name(),
        }
    }
}

impl std::fmt::Display for Privilege {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A capability a deployment grant gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    Adhoc,
    Assert,
    Schema,
    Monitor,
    Cancel,
    Maintain,
    Configure,
    ReadFiles,
    WriteFiles,
}

impl Capability {
    /// Returns the capability of name `name`.
    pub fn named(name: &str) -> Option<Capability> {
        [
            Self::Adhoc,
            Self::Assert,
            Self::Schema,
            Self::Monitor,
            Self::Cancel,
            Self::Maintain,
            Self::Configure,
            Self::ReadFiles,
            Self::WriteFiles,
        ]
        .into_iter()
        .find(|c| c.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Adhoc => "adhoc",
            Self::Assert => "assert",
            Self::Schema => "schema",
            Self::Monitor => "monitor",
            Self::Cancel => "cancel",
            Self::Maintain => "maintain",
            Self::Configure => "configure",
            Self::ReadFiles => "read_files",
            Self::WriteFiles => "write_files",
        }
    }
}
