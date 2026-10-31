use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::ast::{AdhocItem, Grant as Statement, Privilege};
use adtb_lang::evolve::{
    CallSignatures, DeploymentGrant, GrantRow, Stated, closes_option_cycle, deployment_grant,
    family_query,
};
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::signature::{self, Ids};

use crate::catalog;
use crate::runtime::Runtime;
use crate::write::Txn;

use super::super::bundle::{Row, Stored};
use super::super::registered::{self, Checked, Lacking, Over};
use super::super::{Engine, Error, OWNER_ID, RejectReason};
use super::store::{self, Changes};
use super::tables::{Given, Grant, Tables};
use super::{
    Capability, RoleAuthority, RoleSession, denied, diagnosed, give_capability, require, role_name,
};

/// The signatures of the call targets of `catalog::grants` over one engine, each read once.
pub(super) struct Calls<'e, R: Runtime> {
    e: &'e Engine<R>,
    stored: Arc<Stored>,
    served: Vec<Arc<Checked>>,
    reread: RefCell<BTreeMap<String, Result<Arc<Checked>, String>>>,
    families: RefCell<BTreeMap<String, Option<Ids>>>,
}

impl<'e, R: Runtime> Calls<'e, R> {
    pub(super) fn new(e: &'e Engine<R>) -> Self {
        Calls {
            e,
            stored: e.deployed.lock(&e.rt).clone(),
            served: e.registered.lock(&e.rt).clone(),
            reread: RefCell::new(BTreeMap::new()),
            families: RefCell::new(BTreeMap::new()),
        }
    }

    /// Returns the checked file of the bundle of row `row`: the one served, else the text of its
    /// registration checked again at its pin. `None` when no text of it is held.
    fn checked(&self, row: &Row) -> Option<Result<Arc<Checked>, String>> {
        let name = &row.header.name;
        if let Some(c) = self.served.iter().find(|c| c.name() == name) {
            return Some(Ok(c.clone()));
        }
        let mark = self.stored.deployed.registered.get(&row.id)?;
        let mut reread = self.reread.borrow_mut();
        let got = reread.entry(name.clone()).or_insert_with(|| {
            let over = Over {
                source: &self.e.source,
                schema: &self.e.schema,
                program: &self.e.program,
            };
            registered::reread(&over, &mark.text)
                .map(Arc::new)
                .map_err(|why| why.to_string())
        });
        Some(got.clone())
    }

    /// Returns the signature every live bundle of `family` declaring `query` shares: its ids and
    /// its target at the highest version such a bundle pins.
    ///
    /// Fail with `Invalid`, naming the live bundles of the family, when none declares it, two
    /// differ, or the signature of one with callables cannot be read.
    fn unanimous(&self, family: &str, query: &str) -> Result<(Ids, String), Error> {
        let s = *self.e.settings.lock(&self.e.rt);
        let live: Vec<&Row> = self
            .stored
            .live(self.e.clock(), &s)
            .iter()
            .filter_map(|name| self.stored.bundles.get(name))
            .filter(|r| r.header.family == family)
            .collect();
        let names: Vec<&str> = live.iter().map(|r| r.header.name.as_str()).collect();
        let names = names.join(", ");
        let mut shared: Option<(Ids, u64, String)> = None;
        for row in live {
            let name = &row.header.name;
            let c = match self.checked(row) {
                Some(Ok(c)) => c,
                Some(Err(why)) => {
                    return Err(Error::invalid(format!(
                        "the signature of `{family}::{query}` in live bundle {name} cannot be read: {why}"
                    )));
                }
                None if self.stored.calls_of(row.id).is_empty() => continue,
                None => {
                    return Err(Error::invalid(format!(
                        "the signature of `{family}::{query}` in live bundle {name} cannot be read until it is presented at an open or retired"
                    )));
                }
            };
            let (Some(ids), Some((at, target))) = (c.call_ids(query), c.call_target(query)) else {
                continue;
            };
            shared = match shared {
                Some((held, _, _)) if held != ids => {
                    return Err(Error::invalid(format!(
                        "the live bundles of family {family} declare `{query}` with different signatures: {names}"
                    )));
                }
                Some(s) if s.1 >= at => Some(s),
                _ => Some((ids, at, target)),
            };
        }
        let Some((ids, _, target)) = shared else {
            return Err(Error::invalid(format!(
                "no live bundle of family {family} declares `{query}`; live: [{names}]"
            )));
        };
        Ok((ids, target))
    }
}

impl<R: Runtime> Calls<'_, R> {
    /// Refuse deleting the rows `deleted` of `grants` from `grantee`, naming each live bundle
    /// `grantee` owns that lacks after it what it uses. A deleted call grant or capability other
    /// than `read_files` looks at no bundle.
    ///
    /// Fail with `Invalid`, also for such a bundle with callables whose text cannot be read.
    pub(super) fn refuse_lacking(
        &self,
        text: &str,
        grants: &[Grant],
        deleted: &BTreeSet<i64>,
        grantee: (&str, i64),
    ) -> Result<(), Error> {
        let (name, id) = grantee;
        let counts = |g: &Grant| {
            g.privilege == "read_files" || (g.privilege != "call" && g.target.is_some())
        };
        if !grants.iter().any(|g| deleted.contains(&g.id) && counts(g)) {
            return Ok(());
        }
        let after: Vec<_> = grants
            .iter()
            .filter(|g| !deleted.contains(&g.id))
            .filter_map(Grant::deployed)
            .collect();
        let mut lacking = Lacking::new(&self.e.source.grants, &after);
        let s = *self.e.settings.lock(&self.e.rt);
        let mut lines = Vec::new();
        for bundle in self.stored.live(self.e.clock(), &s) {
            let Some(row) = self.stored.bundles.get(&bundle) else {
                continue;
            };
            if self.stored.owner_of(row) != id {
                continue;
            }
            let c = match self.checked(row) {
                Some(Ok(c)) => c,
                Some(Err(why)) => {
                    lines.push(format!("bundle {bundle} cannot be read: {why}"));
                    continue;
                }
                None if self.stored.calls_of(row.id).is_empty() => continue,
                None => {
                    lines.push(format!(
                        "bundle {bundle} cannot be read until it is presented at an open or retired"
                    ));
                    continue;
                }
            };
            lines.extend(
                lacking
                    .of(&c, name)?
                    .into_iter()
                    .map(|l| format!("bundle {bundle}: {l}")),
            );
        }
        if lines.is_empty() {
            return Ok(());
        }
        Err(Error::invalid(format!(
            "`{text}` leaves live bundles lacking what they use:\n{}",
            lines.join("\n")
        )))
    }
}

impl<R: Runtime> CallSignatures for Calls<'_, R> {
    fn family(&self, target: &str) -> Option<Ids> {
        if let Some(known) = self.families.borrow().get(target) {
            return known.clone();
        }
        let read = || {
            let written = signature::family_signature(target)?;
            let source = catalog::source_at(&self.e.source, written.at)?;
            let schema = source.schema().ok()?;
            signature::signature_ids(&schema, &written).ok()
        };
        let ids = read();
        self.families
            .borrow_mut()
            .insert(target.to_owned(), ids.clone());
        ids
    }

    fn query(&self, bundle: &str, query: &str) -> Option<Ids> {
        let row = self.stored.bundles.get(bundle)?;
        self.checked(row)?.ok()?.call_ids(query)
    }

    fn declares(&self, bundle: &str, query: &str) -> Option<bool> {
        let row = self.stored.bundles.get(bundle)?;
        let calls = self.stored.calls_of(row.id);
        Some(
            calls
                .iter()
                .any(|c| c.name == query && c.kind != "ImportedFn"),
        )
    }
}

/// Returns whether `p` names a relation, whether or not one of that name exists.
pub(super) fn on_relation(p: &Privilege) -> bool {
    matches!(
        p,
        Privilege::Read(..)
            | Privilege::Insert(_)
            | Privilege::Update(..)
            | Privilege::Delete(_)
            | Privilege::Retag(..)
    )
}

fn statement(text: &str) -> Result<Statement, Error> {
    let items = parse_adhoc(text, Options::default())
        .ok()
        .map_err(|ds| Error::invalid(diagnosed(&ds)))?;
    let mut items = items.into_iter();
    match (items.next(), items.next()) {
        (Some(AdhocItem::Grant(g)), None) => Ok(g),
        _ => Err(Error::invalid(format!("`{text}` is not one grant"))),
    }
}

/// One privilege of a grant statement being given.
struct Giving<'a, R: Runtime> {
    e: &'a Engine<R>,
    tx: &'a Txn,
    t: &'a Tables<'a>,
    calls: &'a Calls<'a, R>,
    session: &'a RoleSession<R>,
    grantee: &'a str,
    grant_option: bool,
}

impl<R: Runtime> Giving<'_, R> {
    fn may_not(&self, target: &str) -> Error {
        denied(format!(
            "role `{}` may not grant `call` on `{target}`",
            self.session.name
        ))
    }

    fn new_row<'t>(&'t self, target: &'t str) -> GrantRow<'t> {
        GrantRow {
            grantor: &self.session.name,
            grantee: self.grantee,
            privilege: "call",
            target: Some(target),
            grant_option: self.grant_option,
            dormant: false,
        }
    }

    fn option_covers(&self, rows: &[Grant], target: &str) -> bool {
        let new = self.new_row(target);
        rows.iter()
            .any(|g| g.row().keeps(&self.e.schema, self.calls, &new))
    }

    fn refuse_cycle(&self, rows: &[Grant], target: &str, option: bool) -> Result<(), Error> {
        let held: Vec<GrantRow<'_>> = rows.iter().map(Grant::row).collect();
        if option && closes_option_cycle(&self.e.schema, self.calls, &held, self.new_row(target)) {
            return Err(Error::invalid(format!(
                "`call` on `{target}` with grant option from `{}` to `{}` would close a cycle of grant options",
                self.session.name, self.grantee
            )));
        }
        Ok(())
    }

    fn capability(&self, rows: &mut Vec<Grant>, name: &str, text: &str) -> Result<(), Error> {
        let capability = Capability::named(name)
            .ok_or_else(|| Error::invalid(format!("`{text}` names no capability `{name}`")))?;
        require(rows, self.session, Capability::Schema, false)
            .or_else(|_| require(rows, self.session, capability, true))?;
        self.t.grantee(self.e, self.tx, self.grantee)?;
        give_capability(
            self.e,
            self.tx,
            self.t,
            rows,
            (&self.session.name, self.grantee),
            capability,
            self.grant_option,
        )
    }

    /// Give a grant on a system relation.
    ///
    /// Fail with `Denied` for a grant on `catalog::credentials` by any role but the schema owner,
    /// and with `Invalid` for any privilege but `read`.
    fn relation(&self, rows: &mut Vec<Grant>, stated: &Stated) -> Result<(), Error> {
        let credentials = self.e.schema.ids.relations[&self.t.credentials];
        if stated.names(credentials) && self.session.id.0 != OWNER_ID {
            return Err(denied(format!(
                "only the schema owner grants on `catalog::credentials`, not role `{}`",
                self.session.name
            )));
        }
        let privilege = stated.privilege_name();
        let target = stated.target_text(&self.e.schema).ok_or_else(|| {
            Error::rejected(RejectReason::Internal {
                what: format!("a resolved `{privilege}` names an object of the schema"),
            })
        })?;
        if privilege != "read" {
            return Err(Error::invalid(format!(
                "`{privilege}` on system relation `{target}`: a system relation grants only `read`"
            )));
        }
        self.t.grantee(self.e, self.tx, self.grantee)?;
        let new = GrantRow {
            privilege,
            ..self.new_row(&target)
        };
        let held: Vec<GrantRow<'_>> = rows.iter().map(Grant::row).collect();
        if self.grant_option && closes_option_cycle(&self.e.schema, self.calls, &held, new) {
            return Err(Error::invalid(format!(
                "`{privilege}` on `{target}` with grant option from `{}` to `{}` would close a cycle of grant options",
                self.session.name, self.grantee
            )));
        }
        let given = Given {
            grantor: &self.session.name,
            grantee: self.grantee,
            privilege,
            target: Some(&target),
        };
        self.t
            .give_row(self.e, self.tx, rows, &given, self.grant_option)
    }

    /// Give a call grant on bundle `bundle`, or on its query `query`.
    fn bundle(
        &self,
        rows: &mut Vec<Grant>,
        bundle: &str,
        query: Option<&str>,
    ) -> Result<(), Error> {
        let target = query.map_or_else(|| bundle.to_owned(), |q| format!("{bundle}::{q}"));
        let stored = &self.calls.stored;
        let row = stored.bundles.get(bundle);
        let by = self.session.id.0;
        let owns = by == OWNER_ID || row.is_some_and(|r| stored.owner_of(r) == by);
        if !owns && !self.option_covers(rows, &target) {
            return Err(self.may_not(&target));
        }
        let Some(row) = row else {
            return Err(Error::invalid(format!(
                "`{target}` names bundle {bundle}, which is not registered"
            )));
        };
        if row.retired() {
            return Err(Error::invalid(format!(
                "`{target}` names bundle {bundle}, which is retired"
            )));
        }
        if let Some(q) = query {
            let calls = stored.calls_of(row.id);
            match calls.iter().find(|c| c.name == q) {
                None => {
                    return Err(Error::invalid(format!(
                        "`{target}` names no registered query or transaction of bundle {bundle}"
                    )));
                }
                Some(c) if c.kind == "ImportedFn" => {
                    return Err(Error::invalid(format!(
                        "`{target}` names a schema function the bundle imports; it takes `grant call fn`"
                    )));
                }
                Some(_) => {}
            }
        }
        self.t.grantee(self.e, self.tx, self.grantee)?;
        self.refuse_cycle(rows, &target, self.grant_option)?;
        let given = Given {
            grantor: &self.session.name,
            grantee: self.grantee,
            privilege: "call",
            target: Some(&target),
        };
        self.t
            .give_row(self.e, self.tx, rows, &given, self.grant_option)
    }

    fn family(
        &self,
        rows: &mut Vec<Grant>,
        family: &str,
        query: &str,
        written: Option<&str>,
    ) -> Result<(), Error> {
        let named = format!("{family}::{query}");
        let by = self.session.id.0;
        let owns = by == OWNER_ID || self.calls.stored.deployed.families.get(family) == Some(&by);
        let holds = rows.iter().any(|g| {
            g.grantee == self.session.name
                && g.grantor != g.grantee
                && g.privilege == "call"
                && g.grant_option
                && !g.dormant
                && g.target.as_deref().and_then(family_query) == Some(named.as_str())
        });
        if !owns && !holds {
            return Err(self.may_not(&named));
        }
        let authorised = |refusal: Error| {
            if owns { refusal } else { self.may_not(&named) }
        };
        let (ids, target) = self.calls.unanimous(family, query).map_err(authorised)?;
        if let Some(w) = written
            && self.calls.family(w).as_ref() != Some(&ids)
        {
            return Err(authorised(Error::invalid(format!(
                "`{w}` does not have the ids of the signature the live bundles of family {family} share, `{target}`"
            ))));
        }
        if !owns && !self.option_covers(rows, &target) {
            return Err(self.may_not(&named));
        }
        self.t.grantee(self.e, self.tx, self.grantee)?;
        let held = rows.iter().position(|g| {
            g.kind == "Deployment"
                && g.grantee == self.grantee
                && g.grantor == self.session.name
                && g.privilege == "call"
                && g.target.as_deref().and_then(family_query) == Some(named.as_str())
        });
        let same = held
            .and_then(|i| rows[i].target.clone())
            .filter(|t| self.calls.family(t).as_ref() == Some(&ids));
        let target = same.unwrap_or(target);
        let keeps_option = held.is_some_and(|i| rows[i].grant_option);
        self.refuse_cycle(rows, &target, self.grant_option || keeps_option)?;
        let given = Given {
            grantor: &self.session.name,
            grantee: self.grantee,
            privilege: "call",
            target: Some(&target),
        };
        if let Some(i) = held
            && rows[i].target.as_deref() != Some(&target)
        {
            return self.replace(rows, i, &given);
        }
        self.t
            .give_row(self.e, self.tx, rows, &given, self.grant_option)
    }

    /// Replace the family row at `i`, of another signature, by a row of `given`, keeping its option.
    ///
    /// Fail with `Invalid`, naming them, when rows given under the old signature are left uncovered.
    fn replace(&self, rows: &mut Vec<Grant>, i: usize, given: &Given<'_>) -> Result<(), Error> {
        let option = rows[i].grant_option || self.grant_option;
        let old = rows.remove(i);
        let id = self.t.next_id(self.e, self.tx, self.t.grants)?;
        let new = self.t.deployment_grant(id, given, option);
        let schema = &self.e.schema;
        let uncovered: Vec<String> = rows
            .iter()
            .filter(|o| {
                let row = o.row();
                old.row().supports(schema, self.calls, &row)
                    && !new.row().keeps(schema, self.calls, &row)
                    && !rows.iter().any(|g| g.row().keeps(schema, self.calls, &row))
            })
            .map(Grant::printed)
            .collect();
        if !uncovered.is_empty() {
            return Err(Error::invalid(format!(
                "a regrant of another signature leaves these grants uncovered: {}",
                uncovered.join(", ")
            )));
        }
        rows.push(new);
        Ok(())
    }
}

impl<R: Runtime> RoleAuthority<R> {
    /// Run the deployment grant statement `text` as `session`'s role, with one `Grant` audit row.
    ///
    /// Postconditions:
    /// - A role that may not give a privilege is refused with `Denied` whether or not its target
    ///   exists, and nothing changes. A capability takes `schema` or the capability with grant
    ///   option, a grant on a relation takes `schema`, and a call grant takes the target's owner,
    ///   the schema owner or a row of the role with grant option covering it.
    /// - A grant on `catalog::credentials` by a role other than the schema owner is refused with
    ///   `Denied`.
    /// - A target that is absent or retired, an absent grantee, an imported function, a family
    ///   whose live bundles do not share one signature, a relation of the schema, a write
    ///   privilege on a system relation, and an option cycle are refused with `Invalid`.
    /// - A row already held gains the option and loses none. A family row of equal ids keeps its
    ///   id and version; one of other ids is replaced unless that uncovers rows given under it.
    pub fn grant(&self, session: &RoleSession<R>, text: &str) -> Result<(), Error> {
        let statement = statement(text)?;
        let grantee = role_name(&statement.to.text)?;
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let _open = e.enter()?;
        let _writer = e.bundle_writer.lock(&e.rt);
        store::run(&e, true, &mut |e, tx, id, changes| {
            self.grant_in(e, tx, id, changes, (session, &statement, &grantee, text))
        })
    }

    fn grant_in(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        id: super::TxnId,
        changes: &mut Changes,
        (session, statement, grantee, text): (&RoleSession<R>, &Statement, &str, &str),
    ) -> Result<(), Error> {
        let t = Tables::new(&e.schema, id);
        self.checked(session, &t, e, tx)?;
        let calls = Calls::new(e);
        let before = t.grants(e, tx)?;
        let mut rows = before.clone();
        let giving = Giving {
            e,
            tx,
            t: &t,
            calls: &calls,
            session,
            grantee,
            grant_option: statement.grant_option,
        };
        for p in &statement.privileges {
            if on_relation(p) {
                require(&rows, session, Capability::Schema, false)?;
            }
            match deployment_grant(&e.schema, text, p)
                .map_err(|ds| Error::invalid(diagnosed(&ds)))?
            {
                DeploymentGrant::Capability(c) => giving.capability(&mut rows, &c, text)?,
                DeploymentGrant::Bundle(b) => giving.bundle(&mut rows, &b, None)?,
                DeploymentGrant::Query(b, q) => giving.bundle(&mut rows, &b, Some(&q))?,
                DeploymentGrant::Family {
                    family,
                    query,
                    written,
                } => giving.family(&mut rows, &family, &query, written.as_deref())?,
                DeploymentGrant::Relation(stated) => giving.relation(&mut rows, &stated)?,
            }
        }
        t.write_rows(changes, &before, &rows);
        t.audit(e, changes, &session.name, text.to_owned())
    }
}
