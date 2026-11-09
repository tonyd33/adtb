use super::Xid;

/// The transactions that wrote a version: its inserter, and the transaction that deleted or
/// superseded it, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Writers {
    inserter: Xid,
    deleter: Option<Xid>,
}

impl Writers {
    /// The length of [`Writers::encode`]'s bytes.
    pub const LEN: usize = 16;

    /// Returns the writers of a version `x` inserts.
    pub fn inserted(x: Xid) -> Writers {
        Writers {
            inserter: x,
            deleter: None,
        }
    }

    /// Returns these writers with `x` as the deleter.
    ///
    /// Precondition: `x` is not [`Xid::NONE`].
    pub fn superseded_by(self, x: Xid) -> Writers {
        assert_ne!(x, Xid::NONE, "a deleter is a transaction, never Xid::NONE");
        Writers {
            deleter: Some(x),
            ..self
        }
    }

    /// Returns these writers with no deleter.
    pub fn cleared(self) -> Writers {
        Writers {
            deleter: None,
            ..self
        }
    }

    /// Returns whether `next` may be the successor of a version with these writers: its inserter
    /// is this version's deleter.
    pub fn precedes(&self, next: &Writers) -> bool {
        self.deleter == Some(next.inserter)
    }

    /// Returns the transaction a copy of a version with these writers writes its out-of-line
    /// values as: the inserter.
    ///
    /// Precondition: the inserter committed.
    pub fn copy_writer(&self) -> Xid {
        self.inserter
    }

    pub fn inserted_by(&self, x: Xid) -> bool {
        self.inserter == x
    }

    pub fn deleted_by(&self, x: Xid) -> bool {
        self.deleter == Some(x)
    }

    pub fn deleted(&self) -> bool {
        self.deleter.is_some()
    }

    pub fn inserter(&self) -> Xid {
        self.inserter
    }

    pub fn deleter(&self) -> Option<Xid> {
        self.deleter
    }

    /// Returns the inserter and then the deleter as little-endian `u64`s, the deleter 0 when
    /// there is none.
    pub fn encode(&self) -> [u8; Writers::LEN] {
        let mut bytes = [0; Writers::LEN];
        bytes[..8].copy_from_slice(&self.inserter.0.to_le_bytes());
        bytes[8..].copy_from_slice(&self.deleter.unwrap_or(Xid::NONE).0.to_le_bytes());
        bytes
    }

    /// Returns the writers `bytes` encodes.
    pub fn decode(bytes: &[u8; Writers::LEN]) -> Writers {
        let (inserter, deleter) = bytes.split_at(8);
        let word = |b: &[u8]| Xid(u64::from_le_bytes(b.try_into().expect("8 bytes")));
        let deleter = word(deleter);
        Writers {
            inserter: word(inserter),
            deleter: (deleter != Xid::NONE).then_some(deleter),
        }
    }
}
