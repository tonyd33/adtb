//! The `Log` rows of `catalog::grants`: one for each grant of the source, beside the
//! `Deployment` rows that deployment writes.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::evolve::{
    DeployedGrant, GrantRow, LogGrant, NoCalls, Privilege, closes_option_cycle,
};
use adtb_lang::ids::engine_ctor as k;
use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;

use crate::key::Value;
use crate::mvcc::{Hold, Scope, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::Pager;
use crate::write::{self, Sequences};

use super::Rot;
use super::codec::{
    Codec, GRANT_DORMANT, GRANT_GRANTEE, GRANT_GRANTOR, GRANT_ID, GRANT_KIND, GRANT_OPTION,
    GRANT_PRIVILEGE, GRANT_TARGET, GrantKind, RoleGrant, RoleRow,
};
use super::roots::RootsRow;
use super::values::{self, boolean};
use super::{Broken, Catalog, Change, Error, Migrating, OWNER_ROLE, delete_rows};

/// A log grant whose role did not exist when its row was written. It allows nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dormant {
    pub id: i64,
    pub grantee: String,
    pub privilege: Privilege,
    pub target: String,
}

/// What writing the rows of a grant to a role that does not exist does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grantees {
    Refuse,
    Dormant,
}

/// A row of `catalog::grants` a snapshot sees, as stored and as read.
struct Stored {
    grant: RoleGrant,
    row: Vec<Value>,
}

impl Stored {
    fn log(&self) -> bool {
        self.grant.kind() == GrantKind::Log
    }
}

/// The relation `catalog::grants`, its codec and the position of its one constructor.
struct Shape {
    codec: Codec<RoleGrant>,
    grants: RelId,
    ctor: u32,
}

impl Shape {
    fn of(schema: &Schema) -> Shape {
        let codec = Codec::<RoleGrant>::new(schema);
        let grants = codec.relation();
        let ctor = schema.engine_ctors().role_grant().index();
        Shape {
            codec,
            grants,
            ctor,
        }
    }

    /// Returns the `Log` row of grant `g` with row id `id`.
    fn log_row(&self, schema: &Schema, g: &LogGrant, id: i64) -> Result<Vec<Value>, Error> {
        let kind = Value::Sum {
            ctor: self.codec.tag(GRANT_KIND, k::GRANT_LOG),
            fields: Vec::new(),
        };
        let (_, row) = self.codec.encode(
            k::ROLE_GRANT,
            [
                (GRANT_ID, Value::I64(id)),
                (GRANT_KIND.field(), kind),
                (
                    GRANT_PRIVILEGE,
                    Value::Text(g.privilege.privilege().name().to_owned()),
                ),
                (GRANT_TARGET, values::some(Value::Text(target(schema, g)?))),
                (GRANT_GRANTEE, Value::Text(g.role.clone())),
                (GRANT_GRANTOR, Value::Text(OWNER_ROLE.to_owned())),
                (GRANT_OPTION, boolean(g.grant_option)),
                (GRANT_DORMANT, boolean(g.dormant)),
            ],
        );
        Ok(row)
    }
}

/// Returns the target text of grant `g` over `schema`.
///
/// Fail with [`Error::Internal`] when the grant names an object `schema` lacks.
fn target(schema: &Schema, g: &LogGrant) -> Result<String, Error> {
    g.privilege
        .target_text(schema)
        .ok_or_else(|| Error::from(Broken::GrantTarget(g.clone())))
}

fn log_grant_row<'a>(g: &'a LogGrant, target: &'a str) -> GrantRow<'a> {
    GrantRow {
        grantor: OWNER_ROLE,
        grantee: &g.role,
        privilege: g.privilege.privilege(),
        target: Some(target),
        grant_option: g.grant_option,
        dormant: g.dormant,
    }
}

/// Returns the catalog error of a failed draw from the page of id counters.
fn drawn(e: write::Error) -> Error {
    match e {
        write::Error::Exec(crate::exec::Error::Page(e)) => Error::Page(e),
        write::Error::SequenceExhausted => {
            Error::Refused(vec!["the ids of catalog::grants are spent".to_owned()])
        }
        other => Error::from(Broken::GrantId(other)),
    }
}

impl Catalog {
    /// Returns the rows of `catalog::grants` that the snapshot of `hold` sees, with the writes
    /// of `own` when it is a transaction.
    fn stored_grants<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        seen: (&Hold, Option<Xid>),
    ) -> Result<Vec<Stored>, Error> {
        let codec = Codec::<RoleGrant>::new(&self.schema);
        self.rows_to(pager, txns, seen, codec.relation())?
            .into_iter()
            .map(|(c, row)| {
                Ok(Stored {
                    grant: codec.decode(c, &row)?,
                    row,
                })
            })
            .collect()
    }

    /// Refuse a migration whose revoke from a role of `revoked` leaves that role without a grant
    /// option covering rows it granted, naming those rows, or that gives a grant option closing
    /// a cycle. Judge each against `grants`, the grants at the migration's end.
    pub(super) fn refuse_orphans<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        into: &Catalog,
        grants: &[LogGrant],
        revoked: &BTreeSet<String>,
    ) -> Result<(), Error> {
        let stored = self.stored_grants(m.pager, m.txns, (&m.hold, Some(m.x)))?;
        let rows: Vec<GrantRow<'_>> = stored.iter().map(|s| s.grant.row()).collect();
        let mut why = Vec::new();
        for (s, lost) in stored.iter().zip(&rows) {
            let kept = grants
                .iter()
                .any(|g| g.grant_option && g.row == Some(s.grant.id()));
            if !s.log() || !lost.grant_option || kept || !revoked.contains(lost.grantee) {
                continue;
            }
            for o in rows
                .iter()
                .filter(|o| lost.supports(&self.schema, &NoCalls, o))
            {
                let logged = grants.iter().any(|g| {
                    target(&into.schema, g)
                        .is_ok_and(|t| log_grant_row(g, &t).keeps(&into.schema, &NoCalls, o))
                });
                let deployed = stored
                    .iter()
                    .zip(&rows)
                    .any(|(d, r)| !d.log() && r.keeps(&self.schema, &NoCalls, o));
                if !logged && !deployed {
                    why.push(format!(
                        "revoking `{0}` from `{1}` takes the grant option of the grant of `{0}` on `{2}` that `{1}` gave to `{3}`",
                        lost.privilege,
                        lost.grantee,
                        o.target.unwrap_or(""),
                        o.grantee,
                    ));
                }
            }
        }
        for g in grants.iter().filter(|g| g.grant_option && !g.dormant) {
            let had = stored
                .iter()
                .zip(&rows)
                .any(|(s, r)| Some(s.grant.id()) == g.row && r.grant_option);
            let text = target(&into.schema, g)?;
            let new = log_grant_row(g, &text);
            if !had && closes_option_cycle(&into.schema, &NoCalls, &rows, new) {
                why.push(format!(
                    "`{}` on `{text}` with grant option to `{}` would close a cycle of grant options",
                    new.privilege, g.role
                ));
            }
        }
        if why.is_empty() {
            Ok(())
        } else {
            Err(Error::Refused(why))
        }
    }

    /// Returns the name of each role the snapshot of `hold` sees, with the writes of `own` when
    /// it is a transaction, by id.
    fn roles<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        seen: (&Hold, Option<Xid>),
    ) -> Result<BTreeMap<i64, String>, Error> {
        let codec = Codec::<RoleRow>::new(&self.schema);
        self.rows_to(pager, txns, seen, codec.relation())?
            .into_iter()
            .map(|(c, row)| {
                let (id, name, _) = codec.decode(c, &row)?.into_parts();
                Ok((id, name))
            })
            .collect()
    }

    /// Returns the name of each role of the latest commit, by id.
    pub fn committed_roles<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<BTreeMap<i64, String>, Error> {
        let hold = txns.take(Scope::Every)?;
        let roles = self.roles(pager, txns, (&hold, None));
        drop(hold);
        roles
    }

    /// Returns each `Deployment` row of `catalog::grants` of the latest commit.
    pub fn committed_deployed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Vec<DeployedGrant>, Error> {
        let hold = txns.take(Scope::Every)?;
        let stored = self.stored_grants(pager, txns, (&hold, None));
        drop(hold);
        Ok(stored?
            .into_iter()
            .filter(|s| !s.log())
            .map(|s| s.grant.into_deployed())
            .collect())
    }

    /// Give each grant of the source the id of the `Log` row the latest commit holds for it.
    ///
    /// Fail with [`Error::Corrupt`] when a `Log` row records no grant of the source.
    pub(super) fn tag_grants<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<(), Error> {
        let hold = txns.take(Scope::Every)?;
        let stored = self.stored_grants(pager, txns, (&hold, None));
        drop(hold);
        let shape = Shape::of(&self.schema);
        for s in stored?.into_iter().filter(Stored::log) {
            let id = s.grant.id();
            let at = self.source.grants.iter().position(|g| {
                g.row.is_none()
                    && shape
                        .log_row(&self.schema, g, id)
                        .is_ok_and(|row| row == s.row)
            });
            let Some(at) = at else {
                return Err(Rot::UntaggedGrant(id).into());
            };
            self.source.grants[at].row = Some(id);
        }
        Ok(())
    }

    /// Write, as one transaction, a `Log` row of `catalog::grants` for each grant of the source
    /// that has none, dormant when its role does not exist. Returns each dormant row written,
    /// once the transaction is durable.
    ///
    /// Precondition: no `write::Database` holds the page of id counters.
    pub fn record_grants<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Vec<Dormant>, Error> {
        if self.source.grants.iter().all(|g| g.row.is_some()) {
            return Ok(Vec::new());
        }
        self.counters(pager)?;
        let change = Change::begin(pager, txns, Scope::Every)?;
        let before = self.source.grants.clone();
        let dormant = match self.record_in(change.writes()) {
            Ok(dormant) => dormant,
            Err(e) => return Err(change.abandon(e)),
        };
        match change.commit() {
            Ok(landed) => {
                self.end_landed(pager, txns, landed, &[]);
                Ok(dormant)
            }
            Err(e) => {
                self.source.grants = before;
                Err(e)
            }
        }
    }

    /// Write as `m.x` a `Log` row of `catalog::grants` for each grant of the source that has
    /// none, dormant when its role does not exist, and give each grant its row. Returns each
    /// dormant row written.
    ///
    /// Precondition: the catalog has a page of id counters when a grant has no row.
    pub(super) fn record_in<R: Runtime>(
        &mut self,
        m: &Migrating<'_, R>,
    ) -> Result<Vec<Dormant>, Error> {
        if self.source.grants.iter().all(|g| g.row.is_some()) {
            return Ok(Vec::new());
        }
        let mut grants = self.source.grants.clone();
        let dormant =
            self.write_grants(m, self, &mut grants, &BTreeSet::new(), Grantees::Dormant)?;
        self.source.grants = grants;
        Ok(dormant)
    }

    /// Write as `m.x` the `Log` rows of `grants`, the grants of `into`'s source, over those of
    /// this catalog that `m.x` sees at the snapshot of `m.hold`: keep or update the row of each grant
    /// whose id names one, delete every other `Log` row, and insert a row with a new id for each
    /// other grant, giving it that id. Each row is dormant as its grant is. When `mode` keeps
    /// dormant rows, first make dormant each grant whose role `m.x` does not
    /// see, and record each such role at `into`'s version for [`Catalog::open`]. Returns each
    /// grant made dormant.
    ///
    /// Fail with [`Error::Refused`], having drawn no id, when `mode` refuses and a role of
    /// `named` or of an active grant to insert is one `m.x` does not see.
    pub(super) fn write_grants<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        into: &Catalog,
        grants: &mut [LogGrant],
        named: &BTreeSet<String>,
        mode: Grantees,
    ) -> Result<Vec<Dormant>, Error> {
        let schema = &into.schema;
        let shape = Shape::of(schema);
        let stored = self.stored_grants(m.pager, m.txns, (&m.hold, Some(m.x)))?;
        let old: BTreeMap<i64, &Stored> = stored
            .iter()
            .filter(|s| s.log())
            .map(|s| (s.grant.id(), s))
            .collect();
        let mut kept = BTreeSet::new();
        let fresh: Vec<usize> = (0..grants.len())
            .filter(|&i| {
                !grants[i]
                    .row
                    .is_some_and(|id| old.contains_key(&id) && kept.insert(id))
            })
            .collect();
        let roles: BTreeSet<String> = self
            .roles(m.pager, m.txns, (&m.hold, Some(m.x)))?
            .into_values()
            .collect();
        let mut dormant = Vec::new();
        match mode {
            Grantees::Refuse => {
                let missing: Vec<String> = named
                    .iter()
                    .chain(
                        fresh
                            .iter()
                            .map(|&i| &grants[i])
                            .filter(|g| !g.dormant)
                            .map(|g| &g.role),
                    )
                    .filter(|r| !roles.contains(*r))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|r| format!("a grant names role `{r}`, which does not exist"))
                    .collect();
                if !missing.is_empty() {
                    return Err(Error::Refused(missing));
                }
            }
            Grantees::Dormant => {
                let mut marked = BTreeSet::new();
                for (i, g) in grants.iter_mut().enumerate() {
                    let privilege = g.privilege.privilege();
                    let held = !self.held.is_empty()
                        && self
                            .held
                            .contains(&(g.role.clone(), privilege, target(schema, g)?));
                    if g.dormant || roles.contains(&g.role) && !held {
                        continue;
                    }
                    g.dormant = true;
                    marked.insert(g.role.clone());
                    dormant.push((i, g.role.clone()));
                }
                let marks = marked.into_iter().map(|role| RootsRow::Dormant {
                    version: into.source.version,
                    role,
                });
                (self.roots).insert(m.pager, m.txns, m.x, &self.roots.admit(marks)?)?;
            }
        }
        let mut deleted = Vec::new();
        let mut inserted = Vec::new();
        for g in grants
            .iter()
            .filter(|g| g.row.is_some_and(|id| kept.contains(&id)))
        {
            let s = old[&g.row.expect("a kept grant has a row")];
            let row = shape.log_row(schema, g, s.grant.id())?;
            if row != s.row {
                deleted.push(s.row.clone());
                inserted.push(row);
            }
        }
        deleted.extend(
            old.values()
                .filter(|s| !kept.contains(&s.grant.id()))
                .map(|s| s.row.clone()),
        );
        if !fresh.is_empty() {
            let floor = stored
                .iter()
                .map(|s| s.grant.id())
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(write::Error::SequenceExhausted)
                .map_err(drawn)?;
            let Some(page) = self.sequences() else {
                return Err(Error::from(Broken::NoSequences));
            };
            let mut counters = Sequences::open(m.pager, page)?;
            for i in fresh {
                let id = counters
                    .draw(m.pager, shape.grants.0, floor)
                    .map_err(drawn)?;
                grants[i].row = Some(id);
                inserted.push(shape.log_row(schema, &grants[i], id)?);
            }
        }
        delete_rows(m, into.heap(shape.grants, shape.ctor), &deleted)?;
        for row in &inserted {
            into.insert(m, shape.grants, shape.ctor, row)?;
        }
        dormant
            .into_iter()
            .map(|(i, grantee)| {
                let g = &grants[i];
                Ok(Dormant {
                    id: g.row.expect("every grant has a row once written"),
                    grantee,
                    privilege: g.privilege.privilege(),
                    target: target(schema, g)?,
                })
            })
            .collect()
    }
}
