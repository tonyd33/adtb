use std::collections::BTreeSet;
use std::ops::Bound;

use crate::key::{CtorId, DecodeError, Entry, KeySchema, Value, ovarint};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Shape, Walk};

use super::tree::{BTree, Cursor, Entries, Error, prefix_end};

/// The entries of an index, as its tree sees them, none dead.
struct Encoded<'a> {
    schema: &'a KeySchema,
}

impl Entries for Encoded<'_> {
    fn dead(&mut self, _: &[u8]) -> bool {
        false
    }

    fn key_len(&self, entry: &[u8]) -> Option<usize> {
        let e = self.schema.decode_entry(entry).ok()?;
        Some(self.schema.encode(&e.key).ok()?.len())
    }
}

/// An index over the rows of a set of constructors of one relation variable,
/// holding entries `(key, ctor, tid)` in a B-link tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    tree: BTree,
    schema: KeySchema,
    ctors: BTreeSet<CtorId>,
}

impl Index {
    pub fn create<R: Runtime>(
        pager: &Pager<R>,
        owner: u32,
        schema: KeySchema,
        ctors: BTreeSet<CtorId>,
    ) -> Result<Index, PageError> {
        let tree = BTree::create(pager, owner)?;
        Ok(Index::open(tree, schema, ctors))
    }

    pub fn open(tree: BTree, schema: KeySchema, ctors: BTreeSet<CtorId>) -> Index {
        Index {
            tree,
            schema,
            ctors,
        }
    }

    pub fn tree(&self) -> BTree {
        self.tree
    }

    pub fn schema(&self) -> &KeySchema {
        &self.schema
    }

    pub fn ctors(&self) -> &BTreeSet<CtorId> {
        &self.ctors
    }

    pub fn check_key(&self, page_size: usize, key: &[Value], ctor: CtorId) -> Result<(), Error> {
        assert!(
            self.ctors.contains(&ctor),
            "the constructor belongs to the index"
        );
        check_key(&self.schema, page_size, key, ctor)
    }

    fn encode(&self, entry: &Entry) -> Result<Vec<u8>, Error> {
        assert!(
            self.ctors.contains(&entry.ctor),
            "{:?} is not in the index's constructor set",
            entry.ctor
        );
        self.schema.encode_entry(entry).map_err(Error::Key)
    }

    /// Insert `entry`. Returns false if it is already in the index.
    ///
    /// Panics:
    /// - Its constructor is not in the index's set.
    pub fn insert<R: Runtime>(&self, pager: &Pager<R>, entry: &Entry) -> Result<bool, Error> {
        let mut entries = Encoded {
            schema: &self.schema,
        };
        self.tree
            .insert_with(pager, &self.encode(entry)?, &mut entries)
    }

    /// Delete `entry`. Returns false if it was not in the index.
    ///
    /// Panics:
    /// - Its constructor is not in the index's set.
    #[cfg(any(test, feature = "testing"))]
    pub fn delete<R: Runtime>(&self, pager: &Pager<R>, entry: &Entry) -> Result<bool, Error> {
        self.tree.delete(pager, &self.encode(entry)?)
    }

    /// Delete every entry `keep` refuses, one leaf at a time, calling `between` after each leaf
    /// and ending once it returns false. Returns the number deleted, and whether it stopped early.
    pub fn retain<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut keep: impl FnMut(&Entry) -> bool,
        between: &mut dyn FnMut() -> bool,
    ) -> Result<Walk, Error> {
        let mut bad = None;
        let deleted = self.tree.retain(
            pager,
            |bytes| match self.schema.decode_entry(bytes) {
                Ok(e) => keep(&e),
                Err(e) => {
                    bad.get_or_insert((bytes.to_vec(), e));
                    true
                }
            },
            between,
        )?;
        match bad {
            Some((bytes, e)) => Err(self.undecoded(pager, &bytes, e)),
            None => Ok(deleted),
        }
    }

    /// Returns the failure of a read that met `entry`, which does not decode as `error` says:
    /// the refusal of the leaf holding it, judged under its exclusive latch.
    ///
    /// Preconditions: the caller holds no latch on a page of the tree.
    fn undecoded<R: Runtime>(&self, pager: &Pager<R>, entry: &[u8], error: DecodeError) -> Error {
        match self.tree.refuse_holding(pager, entry, Shape::Frame) {
            Error::Malformed(leaf) => Error::Entry(leaf, error),
            e => e,
        }
    }

    fn decode_all<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut cursor: Cursor,
    ) -> Result<Vec<Entry>, Error> {
        let mut out = Vec::new();
        while let Some(bytes) = cursor.next(pager)? {
            match self.schema.decode_entry(&bytes) {
                Ok(e) => out.push(e),
                Err(e) => return Err(self.undecoded(pager, &bytes, e)),
            }
        }
        Ok(out)
    }

    /// Returns the entries whose encoding starts with `prefix`, in order.
    pub fn scan_prefix<R: Runtime>(
        &self,
        pager: &Pager<R>,
        prefix: &[u8],
    ) -> Result<Vec<Entry>, Error> {
        self.decode_all(pager, self.tree.prefix(prefix))
    }

    /// Returns the entries whose encodings lie between `lo` and `hi`, in order.
    pub fn scan<R: Runtime>(
        &self,
        pager: &Pager<R>,
        lo: Bound<&[u8]>,
        hi: Bound<&[u8]>,
    ) -> Result<Vec<Entry>, Error> {
        self.decode_all(pager, self.tree.range(lo, hi))
    }

    /// Refuse the leaf holding `entry` as a stray. Returns the refusal, the error reaching
    /// the leaf, or [`Error::Malformed`] naming the leaf when it does not hold `entry`.
    ///
    /// Preconditions: the caller holds no latch on a page of the tree, and the index does not hold
    /// `entry`'s constructor.
    pub fn refuse_stray<R: Runtime>(&self, pager: &Pager<R>, entry: &Entry) -> Error {
        match self.schema.encode_entry(entry) {
            Ok(bytes) => self.tree.refuse_holding(pager, &bytes, Shape::Stray),
            Err(e) => Error::Key(e),
        }
    }

    /// Returns every entry of `key`, in order of constructor and TID.
    pub fn probe<R: Runtime>(&self, pager: &Pager<R>, key: &[Value]) -> Result<Vec<Entry>, Error> {
        let prefix = self.schema.encode(key).map_err(Error::Key)?;
        self.scan_prefix(pager, &prefix)
    }

    /// Returns the entries of `key` and constructor `ctor`, in TID order.
    #[cfg(any(test, feature = "testing"))]
    pub fn probe_ctor<R: Runtime>(
        &self,
        pager: &Pager<R>,
        key: &[Value],
        ctor: CtorId,
    ) -> Result<Vec<Entry>, Error> {
        let mut prefix = self.schema.encode(key).map_err(Error::Key)?;
        ovarint::encode(ctor.0, &mut prefix);
        self.scan_prefix(pager, &prefix)
    }

    /// Returns the entries whose keys lie between `lo` and `hi`, in order.
    pub fn range<R: Runtime>(
        &self,
        pager: &Pager<R>,
        lo: Bound<&[Value]>,
        hi: Bound<&[Value]>,
    ) -> Result<Vec<Entry>, Error> {
        let encode = |k: &[Value]| self.schema.encode(k).map_err(Error::Key);
        let lo = match lo {
            Bound::Included(k) => Bound::Included(encode(k)?),
            Bound::Excluded(k) => match prefix_end(&encode(k)?) {
                Bound::Excluded(end) => Bound::Included(end),
                _ => return Ok(Vec::new()),
            },
            Bound::Unbounded => Bound::Unbounded,
        };
        let hi = match hi {
            Bound::Included(k) => prefix_end(&encode(k)?),
            Bound::Excluded(k) => Bound::Excluded(encode(k)?),
            Bound::Unbounded => Bound::Unbounded,
        };
        let cursor = self.tree.range(
            lo.as_ref().map(Vec::as_slice),
            hi.as_ref().map(Vec::as_slice),
        );
        self.decode_all(pager, cursor)
    }
}

/// Refuse with `TooLong` key `key` of constructor `ctor` when its entry in an index whose key
/// schema is `schema`, on pages of `page_size` bytes, passes the entry limit.
pub fn check_key(
    schema: &KeySchema,
    page_size: usize,
    key: &[Value],
    ctor: CtorId,
) -> Result<(), Error> {
    let mut encoded = schema.encode(key).map_err(Error::Key)?;
    ovarint::encode(ctor.0, &mut encoded);
    let len = encoded.len() + 8;
    let max = BTree::max_entry_len(page_size);
    if len > max {
        Err(Error::TooLong { len, max })
    } else {
        Ok(())
    }
}
