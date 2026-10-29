//! The stored holds of pending drops: a roots row for each pair of a pending-dropped object and a
//! bundle holding it, read at open and rewritten as holds are added and released.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use adtb_lang::evolve::Source;
use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;

use crate::heap::{self, FieldType, Heap};
use crate::key::{CtorId, Scalar, Value};
use crate::mvcc::{Scope, Transactions};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::ids::{CatFieldId, CatIndexId, CatRelId, field_id, relation};
use super::readers::{Kind, Pending, Readers, enforcing};
use super::root::Changing;
use super::rows::IndexSpec;
use super::{Catalog, Error, INDEX_ROOT, Migrating, ROOTS, delete_rows, indexes, int, uint};

/// The constructor of a roots row naming a pending-dropped constraint and a bundle holding it.
pub(super) const HELD_CONSTRAINT: u32 = 6;
/// The constructor of a roots row naming a pending-dropped check and a bundle holding it.
pub(super) const HELD_CHECK: u32 = 7;
/// The constructor of a roots row naming a pending-dropped unique, by its index, and a bundle
/// holding it.
pub(super) const HELD_UNIQUE: u32 = 8;
/// The constructor of a roots row naming a pending-dropped old key, by its index, its relation
/// and its fields, and a bundle holding it.
pub(super) const HELD_KEY: u32 = 9;

/// The first index id of an old key held pending. The log allocates index ids below it.
const HELD_KEY_INDEXES: u64 = 1 << 31;

/// Returns the constructors of the roots rows that hold pending drops.
pub(super) fn held_ctors() -> [heap::CtorType; 4] {
    let int = || FieldType::Scalar(Scalar::I64);
    let text = || FieldType::Scalar(Scalar::Text);
    let held = || vec![int(), text(), text(), text()];
    let ctor = |id: u32, fields: Vec<FieldType>| heap::CtorType {
        id: CtorId(id),
        fields,
    };
    [
        ctor(HELD_CONSTRAINT, held()),
        ctor(HELD_CHECK, held()),
        ctor(HELD_UNIQUE, [held(), vec![int()]].concat()),
        ctor(
            HELD_KEY,
            [held(), vec![int(), int(), FieldType::Scalar(Scalar::Bytes)]].concat(),
        ),
    ]
}

/// Returns whether roots rows of constructor `ctor` hold pending drops.
pub(super) fn holds(ctor: u32) -> bool {
    matches!(ctor, HELD_CONSTRAINT | HELD_CHECK | HELD_UNIQUE | HELD_KEY)
}

/// Returns the index id for an old key held pending beside the old keys `pending` holds.
pub(super) fn held_key_index(pending: &[Pending]) -> CatIndexId {
    let last = pending
        .iter()
        .filter_map(|p| match &p.kind {
            Kind::Key { index, .. } => Some(index.0),
            _ => None,
        })
        .max();
    CatIndexId(last.map_or(HELD_KEY_INDEXES, |id| id + 1))
}

/// Returns the catalog ids of the fields `names` of relation `r` of `schema`, `None` when one of
/// them has none.
pub(super) fn key_fields(schema: &Schema, r: RelId, names: &[String]) -> Option<Vec<CatFieldId>> {
    let def = schema.relation(r).elem_def;
    names.iter().map(|n| field_id(schema, def, 0, n)).collect()
}

/// Returns the names in `schema` of the fields `ids` of relation `r`, `None` when one of them is
/// not a field of it.
fn field_names(schema: &Schema, r: RelId, ids: &[CatFieldId]) -> Option<Vec<String>> {
    let def = schema.relation(r).elem_def;
    ids.iter()
        .map(|id| {
            schema
                .ids
                .fields
                .iter()
                .find(|&(&(d, _, _), &x)| d == def && x == id.0)
                .map(|((_, _, name), _)| name.clone())
        })
        .collect()
}

/// Returns the relation and the field names in `schema` of old key `kind`, `None` when it has no
/// image there or `kind` is not an old key.
fn old_key(schema: &Schema, kind: &Kind) -> Option<(RelId, Vec<String>)> {
    let Kind::Key {
        relation: rid,
        fields,
        ..
    } = kind
    else {
        return None;
    };
    let r = relation(schema, *rid)?;
    Some((r, field_names(schema, r, fields)?))
}

/// Returns why `schema` leaves each old key of `pending` without an image, naming it and its
/// holders.
pub(super) fn imageless_keys(schema: &Schema, pending: &[Pending]) -> Vec<String> {
    pending
        .iter()
        .filter(|p| matches!(p.kind, Kind::Key { .. }) && old_key(schema, &p.kind).is_none())
        .map(|p| {
            let holders: Vec<&str> = p.blocking.iter().map(String::as_str).collect();
            format!(
                "pending-dropped {}, held by {}, has no image: the migration drops its relation or \
                 a field of it",
                p.object(),
                holders.join(", ")
            )
        })
        .collect()
}

/// Returns every index a catalog over `schema` keeps while the drops of `pending` are pending:
/// the indexes of `schema` and the index of each old key `pending` holds.
///
/// Fail when an old key of `pending` has no image in `schema`.
pub(super) fn kept_indexes(
    schema: &Schema,
    pending: &[Pending],
) -> Result<BTreeMap<u64, IndexSpec>, String> {
    let mut out = indexes(schema);
    for p in pending {
        let Kind::Key { index, .. } = &p.kind else {
            continue;
        };
        let (r, fields) = old_key(schema, &p.kind)
            .ok_or_else(|| format!("pending-dropped {} has no image", p.object()))?;
        out.insert(index.0, IndexSpec::key(schema, r, index.0, fields));
    }
    Ok(out)
}

fn field_bytes(ids: &[CatFieldId]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.0.to_le_bytes()).collect()
}

fn bytes_fields(bytes: &[u8]) -> Option<Vec<CatFieldId>> {
    let (chunks, rest) = bytes.as_chunks::<8>();
    rest.is_empty().then(|| {
        chunks
            .iter()
            .map(|c| CatFieldId(u64::from_le_bytes(*c)))
            .collect()
    })
}

/// Add the hold roots row `fields` of constructor `ctor`, one [`holds`] accepts, names to
/// `pending`.
pub(super) fn add(pending: &mut Vec<Pending>, ctor: u32, fields: &[Value]) -> Result<(), Error> {
    let corrupt = || {
        Error::Corrupt(format!(
            "a roots row of constructor {ctor} holds {fields:?}"
        ))
    };
    let (
        [
            dropped,
            Value::Text(module),
            Value::Text(name),
            Value::Text(holder),
        ],
        index,
    ) = fields.split_at_checked(4).ok_or_else(corrupt)?
    else {
        return Err(corrupt());
    };
    let kind = match (ctor, index) {
        (HELD_CONSTRAINT, []) => Kind::Constraint,
        (HELD_CHECK, []) => Kind::Check,
        (HELD_UNIQUE, [id]) => Kind::Unique(CatIndexId(uint(id)?)),
        (HELD_KEY, [index, rel, Value::Bytes(fields)]) => Kind::Key {
            index: CatIndexId(uint(index)?),
            relation: CatRelId(uint(rel)?),
            fields: bytes_fields(fields).ok_or_else(corrupt)?,
        },
        _ => return Err(corrupt()),
    };
    let dropped = uint(dropped)?;
    let same = |p: &&mut Pending| p.kind == kind && p.module == *module && p.name == *name;
    match pending.iter_mut().find(same) {
        Some(p) if p.dropped != dropped => {
            return Err(Error::Corrupt(format!(
                "{module}::{name} is held as dropped at versions {} and {dropped}",
                p.dropped
            )));
        }
        Some(p) => {
            p.blocking.insert(holder.clone());
        }
        None => pending.push(Pending {
            module: module.clone(),
            name: name.clone(),
            kind,
            dropped,
            blocking: BTreeSet::from([holder.clone()]),
        }),
    }
    Ok(())
}

/// Returns the roots rows holding each drop of `pending` for each of its holders.
fn stored(pending: &[Pending]) -> Vec<Vec<Value>> {
    let mut out = Vec::new();
    for p in pending {
        let ctor = match &p.kind {
            Kind::Constraint => HELD_CONSTRAINT,
            Kind::Check => HELD_CHECK,
            Kind::Unique(_) => HELD_UNIQUE,
            Kind::Key { .. } => HELD_KEY,
        };
        for holder in &p.blocking {
            let mut fields = vec![
                int(p.dropped),
                Value::Text(p.module.clone()),
                Value::Text(p.name.clone()),
                Value::Text(holder.clone()),
            ];
            match &p.kind {
                Kind::Unique(id) => fields.push(int(id.0)),
                Kind::Key {
                    index,
                    relation,
                    fields: key,
                } => fields.extend([
                    int(index.0),
                    int(relation.0),
                    Value::Bytes(field_bytes(key)),
                ]),
                Kind::Constraint | Kind::Check => {}
            }
            out.push(vec![Value::Sum { ctor, fields }]);
        }
    }
    out
}

/// Write as `m.x` the roots rows of `roots` that take the holds of `was` to those of `now`.
pub(super) fn rewrite<R: Runtime>(
    m: &Migrating<'_, R>,
    roots: &Heap,
    was: &[Pending],
    now: &[Pending],
) -> Result<(), Error> {
    let before = stored(was);
    let after = stored(now);
    let gone: Vec<Vec<Value>> = before
        .iter()
        .filter(|r| !after.contains(r))
        .cloned()
        .collect();
    delete_rows(m, roots, &gone)?;
    for row in after.iter().filter(|r| !before.contains(r)) {
        roots.insert(m.pager, m.x.0, row)?;
    }
    Ok(())
}

impl Readers {
    /// Returns the readers of a catalog at `source` as it opens: none live, the drops of
    /// `pending` held by the bundles stored with them, and the source that keeps every
    /// pending-dropped object.
    ///
    /// Fail as [`enforcing`] does.
    pub(super) fn reopened(source: &Source, pending: Vec<Pending>) -> Result<Readers, Error> {
        let enforced = enforcing(source, &pending)?.map(Rc::new);
        Ok(Readers {
            live: Default::default(),
            pending,
            enforced,
        })
    }
}

impl Catalog {
    /// Release the live reader of `bundle`. Every pending drop it was the last to hold takes
    /// effect, and the index of each such unique or old key is freed. Returns once the release is
    /// durable.
    ///
    /// Fail with [`Error::Refused`] when no reader of `bundle` is live, and as [`enforcing`]
    /// does for the drops left pending.
    pub fn release<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        bundle: &str,
    ) -> Result<(), Error> {
        if !self.readers.live.contains_key(bundle) {
            return Err(Error::Refused(vec![format!("bundle {bundle} is not live")]));
        }
        self.unhold(pager, txns, &BTreeSet::from([bundle.to_string()]))?;
        self.readers.live.remove(bundle);
        Ok(())
    }

    /// Release every hold of a pending drop by a bundle that is not a live reader. Every pending
    /// drop left with no holder takes effect, and the index of each such unique or old key is
    /// freed. Returns once the release is durable.
    ///
    /// Fail as [`enforcing`] does for the drops left pending.
    pub fn settle<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<(), Error> {
        let gone: BTreeSet<String> = self
            .readers
            .pending
            .iter()
            .flat_map(|p| &p.blocking)
            .filter(|b| !self.readers.live.contains_key(*b))
            .cloned()
            .collect();
        self.unhold(pager, txns, &gone)
    }

    /// Remove the bundles of `gone` from the holders of every pending drop, and the index of
    /// every unique and old key whose drop takes effect, durably.
    fn unhold<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        gone: &BTreeSet<String>,
    ) -> Result<(), Error> {
        let mut pending = self.readers.pending.clone();
        for p in &mut pending {
            p.blocking.retain(|b| !gone.contains(b));
        }
        if pending == self.readers.pending {
            return Ok(());
        }
        let done = pending.iter().any(|p| p.blocking.is_empty());
        pending.retain(|p| !p.blocking.is_empty());
        let enforced = if done {
            enforcing(&self.source, &pending)?.map(Rc::new)
        } else {
            self.readers.enforced.clone()
        };
        let stored = kept_indexes(
            enforced.as_ref().map_or(&self.schema, |e| &e.schema),
            &pending,
        )
        .map_err(Error::Internal)?;
        let freed: Vec<u64> = self
            .indexes
            .keys()
            .filter(|id| !stored.contains_key(id))
            .copied()
            .collect();
        let x = txns.begin(pager)?;
        if !freed.is_empty() {
            super::update_root(pager, self.root, |r| r.changing = Changing::Schema(x))?;
        }
        let (id, snapshot) = txns.take(Scope::Heaps(BTreeSet::from([ROOTS])))?;
        let m = Migrating {
            pager,
            txns,
            x,
            snapshot: &snapshot,
        };
        let roots: Vec<Vec<Value>> = freed
            .iter()
            .map(|id| {
                let fields = vec![int(*id), int(self.indexes[id].1.tree().root().0)];
                vec![Value::Sum {
                    ctor: INDEX_ROOT,
                    fields,
                }]
            })
            .collect();
        let wrote = rewrite(&m, &self.roots, &self.readers.pending, &pending)
            .and_then(|()| delete_rows(&m, &self.roots, &roots));
        txns.release(id);
        if let Err(e) = wrote {
            txns.abort(pager, x)?;
            if !freed.is_empty() {
                super::update_root(pager, self.root, |r| r.changing = Changing::None)?;
            }
            return Err(e);
        }
        let lsn = txns.commit(pager, x)?;
        pager.wait_durable(lsn);
        self.readers.pending = pending;
        self.readers.enforced = enforced;
        if !freed.is_empty() {
            for id in &freed {
                self.indexes.remove(id);
            }
            self.free_leaked(pager, txns)?;
            let lsn = super::update_root(pager, self.root, |r| r.changing = Changing::None)?;
            pager.wait_durable(lsn);
        }
        Ok(())
    }
}
