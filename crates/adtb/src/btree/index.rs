use std::collections::BTreeSet;
use std::ops::Bound;

use crate::key::{CtorId, CtorType, Dir, Entry, KeySchema, KeyType, SumType, Value, ovarint};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager};

use super::tree::{BTree, Cursor, Entries, Error, prefix_end};

/// The entries of an index, as its tree sees them.
struct Encoded<'a, F> {
    schema: &'a KeySchema,
    dead: &'a mut F,
}

impl<F: FnMut(&Entry) -> bool> Entries for Encoded<'_, F> {
    fn dead(&mut self, entry: &[u8]) -> bool {
        self.schema
            .decode_entry(entry)
            .is_ok_and(|e| (self.dead)(&e))
    }

    fn key_len(&self, entry: &[u8]) -> Option<usize> {
        let e = self.schema.decode_entry(entry).ok()?;
        Some(self.schema.encode(&e.key).len())
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
        let mut encoded = self.schema.encode(key);
        ovarint::encode(ctor.0, &mut encoded);
        let len = encoded.len() + 8;
        let max = BTree::max_entry_len(page_size);
        if len > max {
            Err(Error::TooLong { len, max })
        } else {
            Ok(())
        }
    }

    fn encode(&self, entry: &Entry) -> Vec<u8> {
        assert!(
            self.ctors.contains(&entry.ctor),
            "{:?} is not in the index's constructor set",
            entry.ctor
        );
        self.schema.encode_entry(entry)
    }

    /// Insert `entry`. Returns false if it is already in the index.
    ///
    /// Panics if its constructor is not in the index's set.
    pub fn insert<R: Runtime>(&self, pager: &Pager<R>, entry: &Entry) -> Result<bool, Error> {
        self.insert_with(pager, entry, |_| false)
    }

    /// Insert `entry` as [`Index::insert`] does, first deleting from a leaf
    /// that would otherwise split every entry `dead` judges dead to every
    /// snapshot. The entries of one key in a leaf that would otherwise split
    /// are kept as a posting: the key once, then each constructor and TID.
    ///
    /// `dead` runs under the leaf's exclusive latch: it may read heap pages,
    /// and takes no latch on a page of the index. An entry that does not
    /// decode is kept.
    ///
    /// Panics if its constructor is not in the index's set.
    pub fn insert_with<R: Runtime>(
        &self,
        pager: &Pager<R>,
        entry: &Entry,
        mut dead: impl FnMut(&Entry) -> bool,
    ) -> Result<bool, Error> {
        let mut entries = Encoded {
            schema: &self.schema,
            dead: &mut dead,
        };
        self.tree
            .insert_with(pager, &self.encode(entry), &mut entries)
    }

    /// Delete `entry`. Returns false if it was not in the index.
    ///
    /// Panics if its constructor is not in the index's set.
    pub fn delete<R: Runtime>(&self, pager: &Pager<R>, entry: &Entry) -> Result<bool, Error> {
        self.tree.delete(pager, &self.encode(entry))
    }

    /// Delete every entry `keep` refuses, one leaf at a time. Returns the number deleted.
    pub fn retain<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut keep: impl FnMut(&Entry) -> bool,
    ) -> Result<usize, Error> {
        let mut bad = None;
        let deleted = self
            .tree
            .retain(pager, |bytes| match self.schema.decode_entry(bytes) {
                Ok(e) => keep(&e),
                Err(e) => {
                    bad.get_or_insert(e);
                    true
                }
            })?;
        match bad {
            Some(e) => Err(Error::Entry(e)),
            None => Ok(deleted),
        }
    }

    fn decode_all<R: Runtime>(
        &self,
        pager: &Pager<R>,
        mut cursor: Cursor,
    ) -> Result<Vec<Entry>, Error> {
        let mut out = Vec::new();
        while let Some(bytes) = cursor.next(pager)? {
            out.push(self.schema.decode_entry(&bytes).map_err(Error::Entry)?);
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

    /// Returns every entry of `key`, in order of constructor and TID.
    pub fn probe<R: Runtime>(&self, pager: &Pager<R>, key: &[Value]) -> Result<Vec<Entry>, Error> {
        self.scan_prefix(pager, &self.schema.encode(key))
    }

    /// Returns the entries of `key` and constructor `ctor`, in TID order.
    pub fn probe_ctor<R: Runtime>(
        &self,
        pager: &Pager<R>,
        key: &[Value],
        ctor: CtorId,
    ) -> Result<Vec<Entry>, Error> {
        let mut prefix = self.schema.encode(key);
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
        let past = |k: &[Value]| prefix_end(&self.schema.encode(k));
        let lo = match lo {
            Bound::Included(k) => Bound::Included(self.schema.encode(k)),
            Bound::Excluded(k) => match past(k) {
                Bound::Excluded(end) => Bound::Included(end),
                _ => return Ok(Vec::new()),
            },
            Bound::Unbounded => Bound::Unbounded,
        };
        let hi = match hi {
            Bound::Included(k) => past(k),
            Bound::Excluded(k) => Bound::Excluded(self.schema.encode(k)),
            Bound::Unbounded => Bound::Unbounded,
        };
        let cursor = self.tree.range(
            lo.as_ref().map(Vec::as_slice),
            hi.as_ref().map(Vec::as_slice),
        );
        self.decode_all(pager, cursor)
    }
}

/// How an index keys a value-level sum field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SumKey {
    /// The whole value.
    Whole,
    /// The constructor's order key only.
    Tag,
    /// One field of one constructor; rows holding another constructor have
    /// no entry.
    Field { ctor: u32, field: usize },
}

/// Returns `sum` with every constructor's fields dropped, the type of its
/// tags.
pub fn tag_type(sum: &SumType) -> SumType {
    let ctors = sum
        .ctors()
        .iter()
        .map(|c| CtorType {
            order_key: c.order_key.clone(),
            fields: Vec::new(),
        })
        .collect();
    SumType::new(ctors).expect("a sum's order keys increase")
}

/// Returns the bytes that start every key whose first component is a value
/// of `sum` built by `ctor`, whether the index keys the whole value or its
/// tag.
pub fn tag_prefix(sum: &SumType, ctor: u32, dir: Dir) -> Vec<u8> {
    let schema = KeySchema::new(vec![(KeyType::Sum(tag_type(sum)), dir)]);
    schema.encode(&[Value::Sum {
        ctor,
        fields: Vec::new(),
    }])
}

impl SumKey {
    /// Returns the type of the key component for a field of type `sum`.
    ///
    /// Panics if a `Field` names a constructor or field `sum` lacks.
    pub fn key_type(&self, sum: &SumType) -> KeyType {
        match *self {
            SumKey::Whole => KeyType::Sum(sum.clone()),
            SumKey::Tag => KeyType::Sum(tag_type(sum)),
            SumKey::Field { ctor, field } => sum.ctors()[ctor as usize].fields[field].clone(),
        }
    }

    /// Returns the key component of the sum value `v`, or `None` when the
    /// index holds no entry for it.
    ///
    /// Panics if `v` is not a sum value.
    pub fn component(&self, v: &Value) -> Option<Value> {
        let Value::Sum { ctor, fields } = v else {
            panic!("{v:?} is not a sum value");
        };
        match *self {
            SumKey::Whole => Some(v.clone()),
            SumKey::Tag => Some(Value::Sum {
                ctor: *ctor,
                fields: Vec::new(),
            }),
            SumKey::Field { ctor: c, field } => (c == *ctor).then(|| fields[field].clone()),
        }
    }
}
