//! The `Log` rows of `catalog::grants`: one for each grant of the source, beside the
//! `Deployment` rows that deployment writes.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::evolve::{LogGrant, OptionEdge, closes_option_cycle, target_covers};
use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;

use crate::key::Value;
use crate::mvcc::{Scope, Snapshot, Transactions};
use crate::runtime::Runtime;
use crate::storage::Pager;
use crate::write::{self, Sequences};

use super::values::{self, boolean, relation, tag};
use super::{Catalog, DORMANT, Error, Migrating, OWNER_ROLE, delete_rows, int, layout};

/// A log grant whose role did not exist when its row was written. It allows nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dormant {
    pub id: i64,
    pub grantee: String,
    pub privilege: String,
    pub target: String,
}

/// What writing the rows of a grant to a role that does not exist does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grantees {
    Refuse,
    Dormant,
}

/// A row of `catalog::grants` a snapshot sees.
struct Stored {
    id: i64,
    log: bool,
    row: Vec<Value>,
}

/// The relation `catalog::grants`, its one constructor and its field names.
struct Shape {
    grants: RelId,
    ctor: u32,
    names: Vec<String>,
}

impl Shape {
    fn of(schema: &Schema) -> Shape {
        let grants = relation(schema, "grants");
        let ctor = schema
            .ty(schema.relation(grants).elem_def)
            .ctor_index("RoleGrant")
            .expect("catalog grants declares RoleGrant");
        let names = layout::row(schema, grants, ctor)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        Shape {
            grants,
            ctor,
            names,
        }
    }

    fn at(&self, name: &str) -> usize {
        self.names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("catalog RoleGrant declares {name}"))
    }

    fn text<'r>(&self, row: &'r [Value], name: &str) -> Result<&'r str, Error> {
        match row.get(self.at(name)) {
            Some(Value::Text(t)) => Ok(t),
            other => Err(Error::Corrupt(format!("a grant row's {name} is {other:?}"))),
        }
    }

    fn target<'r>(&self, row: &'r [Value]) -> Option<&'r str> {
        match row.get(self.at("target")) {
            Some(Value::Sum { fields, .. }) => match fields.first() {
                Some(Value::Text(t)) => Some(t),
                _ => None,
            },
            _ => None,
        }
    }

    /// Returns the `Log` row of grant `g` with row id `id`.
    fn log_row(&self, schema: &Schema, g: &LogGrant, id: i64) -> Result<Vec<Value>, Error> {
        let Value::Sum { ctor: some, .. } = tag(schema, "prelude", "Option", "Some") else {
            unreachable!("a tag is a sum");
        };
        let target = Value::Sum {
            ctor: some,
            fields: vec![Value::Text(target(schema, g)?)],
        };
        let fields = vec![
            ("id", Value::I64(id)),
            ("kind", tag(schema, "catalog", "GrantKind", "Log")),
            (
                "privilege",
                Value::Text(g.privilege.privilege_name().to_owned()),
            ),
            ("target", target),
            ("grantee", Value::Text(g.role.clone())),
            ("grantor", Value::Text(OWNER_ROLE.to_owned())),
            ("grant_option", boolean(schema, g.grant_option)),
            ("dormant", boolean(schema, g.dormant)),
        ];
        Ok(values::row(schema, self.grants, self.ctor, fields))
    }
}

/// Returns the target text of grant `g` over `schema`.
///
/// Fail with [`Error::Internal`] when the grant names an object `schema` lacks.
fn target(schema: &Schema, g: &LogGrant) -> Result<String, Error> {
    g.privilege
        .target_text(schema)
        .ok_or_else(|| Error::Internal(format!("a log grant names live objects: {g:?}")))
}

/// Returns the catalog error of a failed draw from the page of id counters.
fn drawn(e: write::Error) -> Error {
    match e {
        write::Error::Page(e) => Error::Page(e),
        write::Error::SequenceExhausted => {
            Error::Refused(vec!["the ids of catalog::grants are spent".to_owned()])
        }
        other => Error::Internal(format!("drawing a grant id: {other:?}")),
    }
}

impl Catalog {
    /// Returns the rows of `catalog::grants` that `snapshot` sees.
    fn stored_grants<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        snapshot: &Snapshot,
    ) -> Result<Vec<Stored>, Error> {
        let shape = Shape::of(&self.schema);
        let (id, kind) = (shape.at("id"), shape.at("kind"));
        let log = tag(&self.schema, "catalog", "GrantKind", "Log");
        self.rows(pager, txns, snapshot, shape.grants)?
            .into_iter()
            .map(|(_, row)| {
                let Some(&Value::I64(at)) = row.get(id) else {
                    return Err(Error::Corrupt("a grant row has no I64 id".to_owned()));
                };
                Ok(Stored {
                    id: at,
                    log: row.get(kind) == Some(&log),
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
        let shape = Shape::of(&self.schema);
        let stored = self.stored_grants(m.pager, m.txns, m.snapshot)?;
        let optioned = boolean(&self.schema, true);
        let has_option = |s: &Stored| s.row.get(shape.at("grant_option")) == Some(&optioned);
        let mut why = Vec::new();
        for s in stored.iter().filter(|s| s.log && has_option(s)) {
            let grantee = shape.text(&s.row, "grantee")?;
            let kept = grants.iter().any(|g| g.grant_option && g.row == Some(s.id));
            if kept || !revoked.contains(grantee) || grantee == OWNER_ROLE {
                continue;
            }
            let privilege = shape.text(&s.row, "privilege")?;
            let held = shape.target(&s.row);
            for o in &stored {
                let given = shape.target(&o.row);
                if shape.text(&o.row, "grantor")? != grantee
                    || shape.text(&o.row, "privilege")? != privilege
                    || !target_covers(&self.schema, privilege, held, given)
                {
                    continue;
                }
                let logged = grants.iter().any(|g| {
                    g.grant_option
                        && !g.dormant
                        && g.role == grantee
                        && g.privilege.privilege_name() == privilege
                        && target(&into.schema, g)
                            .is_ok_and(|t| target_covers(&into.schema, privilege, Some(&t), given))
                });
                let mut deployed = false;
                for d in stored.iter().filter(|d| !d.log && has_option(d)) {
                    deployed |= shape.text(&d.row, "grantee")? == grantee
                        && shape.text(&d.row, "grantor")? != grantee
                        && shape.text(&d.row, "privilege")? == privilege
                        && target_covers(&self.schema, privilege, shape.target(&d.row), given);
                }
                if !logged && !deployed {
                    why.push(format!(
                        "revoking `{privilege}` from `{grantee}` takes the grant option of the grant of `{privilege}` on `{}` that `{grantee}` gave to `{}`",
                        given.unwrap_or(""),
                        shape.text(&o.row, "grantee")?,
                    ));
                }
            }
        }
        let mut edges = Vec::new();
        for s in stored.iter().filter(|s| has_option(s)) {
            edges.push(OptionEdge {
                grantor: shape.text(&s.row, "grantor")?,
                grantee: shape.text(&s.row, "grantee")?,
                privilege: shape.text(&s.row, "privilege")?,
                target: shape.target(&s.row),
            });
        }
        for g in grants.iter().filter(|g| g.grant_option && !g.dormant) {
            let had = stored.iter().any(|s| Some(s.id) == g.row && has_option(s));
            let text = target(&into.schema, g)?;
            let new = OptionEdge {
                grantor: OWNER_ROLE,
                grantee: &g.role,
                privilege: g.privilege.privilege_name(),
                target: Some(&text),
            };
            if !had && closes_option_cycle(&into.schema, &edges, new) {
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

    /// Returns the name of each role `snapshot` sees, by id.
    pub fn roles<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        snapshot: &Snapshot,
    ) -> Result<BTreeMap<i64, String>, Error> {
        let roles = relation(&self.schema, "roles");
        let mut out = BTreeMap::new();
        for (c, row) in self.rows(pager, txns, snapshot, roles)? {
            let fields = layout::row(&self.schema, roles, c);
            let at = |name: &str| {
                fields
                    .iter()
                    .position(|(n, _)| n == name)
                    .unwrap_or_else(|| panic!("catalog Role declares {name}"))
            };
            match (row.get(at("id")), row.get(at("name"))) {
                (Some(Value::I64(id)), Some(Value::Text(name))) => {
                    out.insert(*id, name.clone());
                }
                _ => return Err(Error::Corrupt(format!("a role row is {row:?}"))),
            }
        }
        Ok(out)
    }

    /// Returns the name of each role of the latest commit, by id.
    pub fn committed_roles<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<BTreeMap<i64, String>, Error> {
        let (id, snapshot) = txns.take(Scope::Every)?;
        let roles = self.roles(pager, txns, &snapshot);
        txns.release(id);
        roles
    }

    /// Give each grant of the source the id of the `Log` row the latest commit holds for it.
    ///
    /// Fail with [`Error::Corrupt`] when a `Log` row records no grant of the source.
    pub(super) fn tag_grants<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<(), Error> {
        let (id, snapshot) = txns.take(Scope::Every)?;
        let stored = self.stored_grants(pager, txns, &snapshot);
        txns.release(id);
        let shape = Shape::of(&self.schema);
        for s in stored?.into_iter().filter(|s| s.log) {
            let at = self.source.grants.iter().position(|g| {
                g.row.is_none()
                    && shape
                        .log_row(&self.schema, g, s.id)
                        .is_ok_and(|row| row == s.row)
            });
            let Some(at) = at else {
                return Err(Error::Corrupt(format!(
                    "grant row {} records no grant of the log",
                    s.id
                )));
            };
            self.source.grants[at].row = Some(s.id);
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
        let x = txns.begin(pager)?;
        let (id, snapshot) = txns.take(Scope::Every)?;
        let m = Migrating {
            pager,
            txns,
            x,
            snapshot: &snapshot,
        };
        let mut grants = self.source.grants.clone();
        let done = self.write_grants(&m, self, &mut grants, &BTreeSet::new(), Grantees::Dormant);
        txns.release(id);
        match done {
            Ok(dormant) => {
                let lsn = txns.commit(pager, x)?;
                pager.wait_durable(lsn);
                self.source.grants = grants;
                Ok(dormant)
            }
            Err(e) => {
                txns.abort(pager, x)?;
                Err(e)
            }
        }
    }

    /// Write as `m.x` the `Log` rows of `grants`, the grants of `into`'s source, over those of
    /// this catalog that `m.snapshot` sees: keep or update the row of each grant whose id names
    /// one, delete every other `Log` row, and insert a row with a new id for each other grant,
    /// giving it that id. Each row is dormant as its grant is. When `mode` keeps dormant rows,
    /// first make dormant each grant whose role `m.snapshot` does not see, and record each such
    /// role at `into`'s version for [`Catalog::open`]. Returns each grant made dormant.
    ///
    /// Fail with [`Error::Refused`], having drawn no id, when `mode` refuses and a role of
    /// `named` or of an active grant to insert is one `m.snapshot` does not see.
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
        let stored = self.stored_grants(m.pager, m.txns, m.snapshot)?;
        let old: BTreeMap<i64, &Stored> =
            stored.iter().filter(|s| s.log).map(|s| (s.id, s)).collect();
        let mut kept = BTreeSet::new();
        let fresh: Vec<usize> = (0..grants.len())
            .filter(|&i| {
                !grants[i]
                    .row
                    .is_some_and(|id| old.contains_key(&id) && kept.insert(id))
            })
            .collect();
        let roles: BTreeSet<String> = self
            .roles(m.pager, m.txns, m.snapshot)?
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
                    if g.dormant || roles.contains(&g.role) {
                        continue;
                    }
                    g.dormant = true;
                    marked.insert(g.role.clone());
                    dormant.push((i, g.role.clone()));
                }
                for role in marked {
                    let fields = vec![int(into.source.version), Value::Text(role)];
                    let mark = Value::Sum {
                        ctor: DORMANT,
                        fields,
                    };
                    self.roots.insert(m.pager, m.x.0, &[mark])?;
                }
            }
        }
        let mut deleted = Vec::new();
        let mut inserted = Vec::new();
        for g in grants
            .iter()
            .filter(|g| g.row.is_some_and(|id| kept.contains(&id)))
        {
            let s = old[&g.row.expect("a kept grant has a row")];
            let row = shape.log_row(schema, g, s.id)?;
            if row != s.row {
                deleted.push(s.row.clone());
                inserted.push(row);
            }
        }
        deleted.extend(
            old.values()
                .filter(|s| !kept.contains(&s.id))
                .map(|s| s.row.clone()),
        );
        if !fresh.is_empty() {
            let floor = stored
                .iter()
                .map(|s| s.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(write::Error::SequenceExhausted)
                .map_err(drawn)?;
            let Some(page) = self.sequences() else {
                return Err(Error::Internal(
                    "a grant row id needs a named page of id counters".to_owned(),
                ));
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
            into.insert(m.pager, m.x, shape.grants, shape.ctor, row)?;
        }
        dormant
            .into_iter()
            .map(|(i, grantee)| {
                let g = &grants[i];
                Ok(Dormant {
                    id: g.row.expect("every grant has a row once written"),
                    grantee,
                    privilege: g.privilege.privilege_name().to_owned(),
                    target: target(schema, g)?,
                })
            })
            .collect()
    }
}
