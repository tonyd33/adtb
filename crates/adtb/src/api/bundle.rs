//! Bundles: the header of a bundle file, the rows of `catalog::bundles` and `catalog::registered`
//! that hold their deployment state, leases, retirement and supersession.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ast::{self, BundleHeader, BundleName, ExprKind, HeaderClause, Lit};
use adtb_lang::check::Command;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::{RelId, Ty};

use crate::catalog::{Catalog, Deployed, Fact, Mark, Origin, Rewrite, layout};
use crate::exec::{self, Val};
use crate::key::Value;
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::error::{Error, RejectReason};
use super::settings::Settings;
use super::text::{DynRows, rows};
use super::{Engine, OWNER_ID, OpenError, corrupt};

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;

/// The edition of the specification this engine implements.
pub(super) const EDITION: &str = "2026-10";

/// The header of a bundle file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Header {
    pub(super) name: String,
    pub(super) family: String,
    pub(super) version: String,
    pub(super) pins: u64,
    /// The pinned version's hash, 64 lowercase hex digits.
    pub(super) hash: String,
    pub(super) catalog: Option<u64>,
    pub(super) edition: String,
    /// The lease the header states, in microseconds.
    pub(super) lease: Option<i64>,
    pub(super) hides_new: bool,
    pub(super) priority: exec::Priority,
}

impl Header {
    /// Returns the header `h` states, refusing a clause this engine does not run.
    pub(super) fn of(h: &BundleHeader) -> Result<Header, Error> {
        let hash =
            h.pins.hash.as_ref().ok_or_else(|| {
                Error::invalid("a bundle header states its pinned version's hash")
            })?;
        let mut header = Header {
            name: format!("{}-{}", h.name.family, h.name.version),
            family: h.name.family.clone(),
            version: h.name.version.clone(),
            pins: h.pins.number,
            hash: hash.to_ascii_lowercase(),
            catalog: h.catalog.as_ref().map(|v| v.number),
            edition: h.edition.clone(),
            lease: None,
            hides_new: false,
            priority: exec::Priority::Regular,
        };
        for clause in &h.clauses {
            match clause {
                HeaderClause::Lease(e) => header.lease = Some(lease(e)?),
                HeaderClause::Priority(p) => {
                    header.priority = priority(&p.text)
                        .filter(|c| *c != exec::Priority::AdHoc)
                        .ok_or_else(|| {
                            Error::invalid(format!(
                                "a bundle's priority is background or regular, not `{}`",
                                p.text
                            ))
                        })?;
                }
                HeaderClause::HideNewConstructors => header.hides_new = true,
                HeaderClause::Budget(_) => {
                    return Err(super::unsupported("a bundle's budget clause"));
                }
            }
        }
        Ok(header)
    }

    /// Returns the lease duration, in microseconds, under the file settings `s`.
    fn lease(&self, s: &Settings) -> i64 {
        self.lease.unwrap_or(s.lease)
    }
}

/// Returns the microseconds of the duration literal `e`.
fn lease(e: &ast::Expr) -> Result<i64, Error> {
    let ExprKind::Lit(Lit::Duration { amount, unit }) = &e.kind else {
        return Err(Error::invalid("a lease is a duration literal"));
    };
    ast::duration_unit(unit)
        .and_then(|per| i64::try_from(*amount).ok()?.checked_mul(per))
        .filter(|&d| d > 0)
        .ok_or_else(|| Error::invalid(format!("`{amount}{unit}` is no lease")))
}

/// Each priority class with its constructor of `catalog::Priority`.
const PRIORITIES: [(&str, exec::Priority); 3] = [
    ("AdHoc", exec::Priority::AdHoc),
    ("Background", exec::Priority::Background),
    ("Regular", exec::Priority::Regular),
];

/// Returns the priority class the word `p` names, in any case.
fn priority(p: &str) -> Option<exec::Priority> {
    PRIORITIES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(p))
        .map(|(_, c)| *c)
}

/// Returns the constructor of `catalog::Priority` that names class `c`.
fn priority_name(c: exec::Priority) -> &'static str {
    PRIORITIES
        .iter()
        .find(|(_, x)| *x == c)
        .map(|(name, _)| *name)
        .expect("every priority class has a constructor")
}

/// The state of a bundle's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Active,
    Superseded(i64),
    Retired(i64),
}

/// A row of `catalog::bundles`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) id: i64,
    pub(super) header: Header,
    pub(super) owner: String,
    /// The last renewal of the lease, in microseconds since the epoch.
    pub(super) renewed: i64,
    pub(super) state: State,
}

impl Row {
    /// Returns whether the lease has run out at `now` under the file settings `s`, with the
    /// primary's grace.
    fn expired(&self, now: i64, s: &Settings) -> bool {
        now >= self.renewed + self.header.lease(s) + s.grace
    }

    /// Returns whether the bundle is live at `now` under the file settings `s`: neither retired,
    /// superseded nor expired.
    fn live(&self, now: i64, s: &Settings) -> bool {
        self.state == State::Active && !self.expired(now, s)
    }

    /// Returns whether a bind or call at `now` writes a renewal: an eighth of the lease has
    /// passed since the last, and the lease has not run out.
    fn renewal_due(&self, now: i64, s: &Settings) -> bool {
        let lease = self.header.lease(s);
        now - self.renewed >= lease / 8 && now < self.renewed + lease
    }

    /// Returns why a bind or call of the bundle is refused at `now`, the call's query named
    /// `query`, if it is.
    fn refusal(&self, now: i64, s: &Settings, query: &str) -> Option<RejectReason> {
        let bundle = self.header.name.clone();
        match self.state {
            State::Retired(_) => Some(RejectReason::Retired { bundle }),
            State::Superseded(_) => Some(RejectReason::BundleSuperseded { bundle }),
            State::Active if self.expired(now, s) => Some(RejectReason::BundleExpired {
                bundle,
                query: query.to_owned(),
            }),
            State::Active => None,
        }
    }
}

/// A bundle whose registration passed every check: its header, calls and file text.
pub(super) struct Passed<'a> {
    pub(super) header: &'a Header,
    pub(super) calls: Vec<CallRow>,
    pub(super) text: &'a str,
}

/// One registered call of a bundle, as a row of `catalog::registered` holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CallRow {
    /// The constructor of `catalog::Registered`: `Query`, `Txn` or `ImportedFn`.
    pub(super) kind: &'static str,
    pub(super) name: String,
    pub(super) ordinal: i64,
    pub(super) signature: String,
    pub(super) text: String,
}

/// The rows of `catalog::bundles` and `catalog::registered`, by bundle name.
#[derive(Clone, Debug, Default)]
pub(super) struct Stored {
    pub(super) bundles: BTreeMap<String, Row>,
    /// The registered calls of each bundle, by bundle id, with their row ids.
    calls: BTreeMap<i64, Vec<(i64, CallRow)>>,
    pub(super) deployed: Deployed,
}

impl Stored {
    /// Returns the name of each bundle live at `now` under the file settings `s`.
    pub(super) fn live(&self, now: i64, s: &Settings) -> BTreeSet<String> {
        self.bundles
            .values()
            .filter(|r| r.live(now, s))
            .map(|r| r.header.name.clone())
            .collect()
    }

    /// Returns the text of each live bundle at `now`, under the file settings `s`, that a role
    /// registered by the command, by name.
    pub(super) fn commanded(&self, now: i64, s: &Settings) -> BTreeMap<String, String> {
        self.bundles
            .values()
            .filter(|r| r.live(now, s))
            .filter_map(|r| {
                let m = self.deployed.registered.get(&r.id)?;
                (m.origin == Origin::Command).then(|| (r.header.name.clone(), m.text.clone()))
            })
            .collect()
    }

    pub(super) fn next_bundle(&self) -> i64 {
        self.bundles.values().map(|r| r.id).max().unwrap_or(0) + 1
    }

    pub(super) fn next_call(&self) -> i64 {
        self.calls
            .values()
            .flatten()
            .map(|(id, _)| *id)
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Returns the calls stored for bundle `id`, without their row ids.
    fn calls_of(&self, id: i64) -> Vec<&CallRow> {
        self.calls
            .get(&id)
            .map(|cs| cs.iter().map(|(_, c)| c).collect())
            .unwrap_or_default()
    }
}

/// The relations and types the deployment rows live in.
struct Tables<'s> {
    schema: &'s Schema,
    bundles: RelId,
    registered: RelId,
}

impl<'s> Tables<'s> {
    fn of(schema: &'s Schema) -> Tables<'s> {
        Tables {
            schema,
            bundles: catalog_relation(schema, "bundles"),
            registered: catalog_relation(schema, "registered"),
        }
    }

    /// Returns the constructor of catalog type `ty` named `name`.
    fn ctor(&self, ty: &str, name: &str) -> u32 {
        let s = self.schema;
        let m = s
            .module_named("catalog")
            .expect("the engine declares module `catalog`");
        let def = s.module(m).types[ty];
        s.ty(def)
            .ctor_index(name)
            .unwrap_or_else(|| panic!("`catalog::{ty}` has constructor `{name}`"))
    }

    fn ctor_name(&self, ty: &str, c: u32) -> String {
        let s = self.schema;
        let m = s
            .module_named("catalog")
            .expect("the engine declares module `catalog`");
        s.ty(s.module(m).types[ty]).ctors()[c as usize].name.clone()
    }

    /// Returns the value of `row`.
    fn bundle_value(&self, row: &Row) -> Val {
        let h = &row.header;
        let state = match row.state {
            State::Active => Val::ctor(self.ctor("BundleState", "Active"), Vec::new()),
            State::Superseded(at) => Val::ctor(
                self.ctor("BundleState", "Superseded"),
                vec![Val::Instant(at)],
            ),
            State::Retired(at) => {
                Val::ctor(self.ctor("BundleState", "Retired"), vec![Val::Instant(at)])
            }
        };
        let fields = BTreeMap::from([
            ("id", Val::Int(row.id)),
            ("name", Val::text(&h.name)),
            ("family", Val::text(&h.family)),
            ("owner", Val::text(&row.owner)),
            ("pins", Val::Int(int(h.pins))),
            ("hash", Val::Bytes(unhex(&h.hash).into())),
            ("edition", Val::text(&h.edition)),
            (
                "catalog",
                h.catalog
                    .map_or_else(Val::none, |c| Val::some(Val::Int(int(c)))),
            ),
            ("hides_new", Val::bool(h.hides_new)),
            (
                "lease",
                h.lease
                    .map_or_else(Val::none, |l| Val::some(Val::Duration(l))),
            ),
            ("renewed", Val::Instant(row.renewed)),
            (
                "priority",
                Val::ctor(self.ctor("Priority", priority_name(h.priority)), Vec::new()),
            ),
            ("memory_share", Val::none()),
            ("workers", Val::none()),
            ("state", state),
        ]);
        Val::ctor(0, self.ordered(self.bundles, 0, &fields))
    }

    /// Returns the row `v`, a value of `catalog::Bundle`.
    fn bundle_row(&self, v: &Val) -> Result<Row, String> {
        let f = self.named(self.bundles, v)?;
        let text = |n: &str| match f.get(n) {
            Some(Val::Text(t)) => Ok(t.to_string()),
            other => Err(format!("field `{n}` of a bundle row is {other:?}")),
        };
        let number = |n: &str| match f.get(n) {
            Some(Val::Int(i)) => Ok(*i),
            other => Err(format!("field `{n}` of a bundle row is {other:?}")),
        };
        let some = |n: &str| match f.get(n) {
            Some(Val::Ctor(1, inner)) => Ok(inner.first().cloned()),
            Some(Val::Ctor(0, _)) => Ok(None),
            other => Err(format!("field `{n}` of a bundle row is {other:?}")),
        };
        let name = text("name")?;
        let state = match f.get("state") {
            Some(Val::Ctor(c, at)) => {
                match (self.ctor_name("BundleState", *c).as_str(), at.first()) {
                    ("Active", _) => State::Active,
                    ("Superseded", Some(Val::Instant(t))) => State::Superseded(*t),
                    ("Retired", Some(Val::Instant(t))) => State::Retired(*t),
                    other => return Err(format!("bundle {name} has state {other:?}")),
                }
            }
            other => return Err(format!("bundle {name} has state {other:?}")),
        };
        let header = Header {
            version: name
                .rsplit_once('-')
                .map(|(_, v)| v.to_owned())
                .unwrap_or_default(),
            family: text("family")?,
            pins: u64::try_from(number("pins")?).map_err(|e| e.to_string())?,
            hash: match f.get("hash") {
                Some(Val::Bytes(b)) => b.iter().map(|x| format!("{x:02x}")).collect(),
                other => return Err(format!("bundle {name} has hash {other:?}")),
            },
            catalog: match some("catalog")? {
                Some(Val::Int(c)) => Some(u64::try_from(c).map_err(|e| e.to_string())?),
                _ => None,
            },
            edition: text("edition")?,
            lease: match some("lease")? {
                Some(Val::Duration(d)) => Some(d),
                _ => None,
            },
            hides_new: matches!(f.get("hides_new"), Some(v) if v.truth().unwrap_or(false)),
            priority: match f.get("priority") {
                Some(Val::Ctor(c, _)) => {
                    let ctor = self.ctor_name("Priority", *c);
                    priority(&ctor).ok_or_else(|| format!("bundle {name} has priority {ctor}"))?
                }
                other => return Err(format!("bundle {name} has priority {other:?}")),
            },
            name,
        };
        Ok(Row {
            id: number("id")?,
            owner: text("owner")?,
            renewed: match f.get("renewed") {
                Some(Val::Instant(t)) => *t,
                other => return Err(format!("field `renewed` of a bundle row is {other:?}")),
            },
            state,
            header,
        })
    }

    /// Returns the value of call `c`, row `id` of bundle `bundle`.
    fn call_value(&self, id: i64, bundle: i64, c: &CallRow) -> Val {
        let kind = self.ctor("Registered", c.kind);
        let fields = BTreeMap::from([
            ("id", Val::Int(id)),
            ("bundle", Val::Int(bundle)),
            ("name", Val::text(&c.name)),
            ("ordinal", Val::Int(c.ordinal)),
            ("signature", Val::text(&c.signature)),
            ("text", Val::text(&c.text)),
            ("limit", Val::none()),
        ]);
        Val::ctor(kind, self.ordered(self.registered, kind, &fields))
    }

    /// Returns the row id, bundle id and call of `v`, a value of `catalog::Registered`.
    fn call_row(&self, v: &Val) -> Result<(i64, i64, CallRow), String> {
        let Val::Ctor(kind, _) = v else {
            return Err(format!("a registered row is {v:?}"));
        };
        let f = self.named(self.registered, v)?;
        let text = |n: &str| match f.get(n) {
            Some(Val::Text(t)) => Ok(t.to_string()),
            other => Err(format!("field `{n}` of a registered row is {other:?}")),
        };
        let number = |n: &str| match f.get(n) {
            Some(Val::Int(i)) => Ok(*i),
            other => Err(format!("field `{n}` of a registered row is {other:?}")),
        };
        let kind = match self.ctor_name("Registered", *kind).as_str() {
            "Query" => "Query",
            "Txn" => "Txn",
            _ => "ImportedFn",
        };
        Ok((
            number("id")?,
            number("bundle")?,
            CallRow {
                kind,
                name: text("name")?,
                ordinal: number("ordinal")?,
                signature: text("signature")?,
                text: text("text")?,
            },
        ))
    }

    /// Returns the values of `fields` in the order of a row of constructor `c` of `rel`.
    fn ordered(&self, rel: RelId, c: u32, fields: &BTreeMap<&str, Val>) -> Vec<Val> {
        layout::row(self.schema, rel, c)
            .iter()
            .map(|(n, _)| {
                fields
                    .get(n.as_str())
                    .unwrap_or_else(|| panic!("the engine writes field `{n}`"))
                    .clone()
            })
            .collect()
    }

    /// Returns the fields of `v`, a row of `rel`, by name.
    fn named(&self, rel: RelId, v: &Val) -> Result<BTreeMap<String, Val>, String> {
        let Val::Ctor(c, fs) = v else {
            return Err(format!("a catalog row is {v:?}"));
        };
        Ok(layout::row(self.schema, rel, *c)
            .into_iter()
            .map(|(n, _)| n)
            .zip(fs.iter().cloned())
            .collect())
    }

    /// Returns the stored form of `v`, a row of `rel`.
    fn stored(&self, rel: RelId, v: &Val) -> Result<(u32, Vec<Value>), OpenError> {
        let Val::Ctor(c, fs) = v else {
            return Err(corrupt(format!("a catalog row is {v:?}")));
        };
        let fields = layout::row(self.schema, rel, *c)
            .iter()
            .zip(fs.iter())
            .map(|((_, t), v)| exec::to_stored(&layout::field_type(self.schema, t), v))
            .collect::<Result<Vec<_>, _>>()
            .map_err(corrupt)?;
        Ok((*c, fields))
    }

    /// Returns the value of `row`, a stored row of constructor `c` of `rel`.
    fn value(&self, rel: RelId, c: u32, row: &[Value]) -> Val {
        let fields = layout::row(self.schema, rel, c)
            .iter()
            .zip(row)
            .map(|((_, t), v)| exec::from_stored(&exec::conv(self.schema, t), v))
            .collect();
        Val::ctor(c, fields)
    }
}

/// Returns the relation `catalog::name` of `schema`.
fn catalog_relation(schema: &Schema, name: &str) -> RelId {
    let m = schema
        .module_named("catalog")
        .expect("the engine declares module `catalog`");
    match schema.module(m).values.get(name) {
        Some(ValueDef::Relation(r)) => *r,
        _ => panic!("the engine declares `catalog::{name}`"),
    }
}

fn int(n: u64) -> i64 {
    i64::try_from(n).expect("a version fits 63 bits")
}

/// Returns the bytes of the hex digits `h`.
fn unhex(h: &str) -> Vec<u8> {
    (0..h.len() / 2)
        .filter_map(|i| u8::from_str_radix(h.get(2 * i..2 * i + 2)?, 16).ok())
        .collect()
}

/// Returns the rows of `catalog::bundles` and `catalog::registered` the latest commit holds.
pub(super) fn stored<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
) -> Result<Stored, OpenError> {
    let t = Tables::of(catalog.schema());
    let mut out = Stored::default();
    for (c, row) in catalog
        .committed_rows(pager, txns, t.bundles)
        .map_err(corrupt)?
    {
        let row = t
            .bundle_row(&t.value(t.bundles, c, &row))
            .map_err(corrupt)?;
        out.bundles.insert(row.header.name.clone(), row);
    }
    for (c, row) in catalog
        .committed_rows(pager, txns, t.registered)
        .map_err(corrupt)?
    {
        let (id, bundle, call) = t
            .call_row(&t.value(t.registered, c, &row))
            .map_err(corrupt)?;
        out.calls.entry(bundle).or_default().push((id, call));
    }
    for calls in out.calls.values_mut() {
        calls.sort_by_key(|(_, c)| c.ordinal);
    }
    out.deployed = catalog.deployed(pager, txns).map_err(corrupt)?;
    Ok(out)
}

/// The rows one deployment write puts in place: each bundle row with the row it replaces, and
/// the calls it adds, each with its row id and its bundle's id.
#[derive(Default)]
struct Writes {
    bundles: Vec<(Option<Row>, Row)>,
    calls: Vec<(i64, i64, CallRow)>,
    facts: Vec<Fact>,
}

impl Writes {
    /// Write these rows as one durable transaction.
    fn commit<R: Runtime>(
        self,
        catalog: &Catalog,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<(), OpenError> {
        if self.bundles.is_empty() && self.calls.is_empty() && self.facts.is_empty() {
            return Ok(());
        }
        let t = Tables::of(catalog.schema());
        let mut bundles = Rewrite {
            relation: t.bundles,
            deleted: Vec::new(),
            inserted: Vec::new(),
        };
        for (old, new) in &self.bundles {
            if let Some(old) = old {
                bundles
                    .deleted
                    .push(t.stored(t.bundles, &t.bundle_value(old))?);
            }
            bundles
                .inserted
                .push(t.stored(t.bundles, &t.bundle_value(new))?);
        }
        let calls = Rewrite {
            relation: t.registered,
            deleted: Vec::new(),
            inserted: self
                .calls
                .iter()
                .map(|(id, b, c)| t.stored(t.registered, &t.call_value(*id, *b, c)))
                .collect::<Result<_, _>>()?,
        };
        catalog
            .deploy_with(pager, txns, &[bundles, calls], &self.facts)
            .map_err(corrupt)
    }
}

/// Record each bundle of `headers` that is not recorded yet, and supersede every active bundle
/// an open registered that they do not name, as one durable transaction. On a file that
/// records no family owners yet, give the schema owner every family it holds a bundle of.
pub(super) fn record<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    stored: &mut Stored,
    headers: &[&Header],
    now: i64,
) -> Result<(), OpenError> {
    let mut writes = Writes::default();
    let mut next = stored.next_bundle();
    for h in headers {
        if !stored.bundles.contains_key(&h.name) {
            let row = Row {
                id: next,
                header: (*h).clone(),
                owner: super::OWNER_ROLE.to_owned(),
                renewed: now,
                state: State::Active,
            };
            next += 1;
            writes.bundles.push((None, row));
        }
    }
    writes.facts = legacy(stored);
    for row in stored.bundles.values() {
        let opened = stored
            .deployed
            .registered
            .get(&row.id)
            .is_none_or(|m| m.origin == Origin::Open);
        if opened
            && row.state == State::Active
            && !headers.iter().any(|h| h.name == row.header.name)
        {
            let mut next = row.clone();
            next.state = State::Superseded(now);
            writes.bundles.push((Some(row.clone()), next));
        }
    }
    let changed: Vec<Row> = writes.bundles.iter().map(|(_, r)| r.clone()).collect();
    let facts = writes.facts.clone();
    writes.commit(catalog, pager, txns)?;
    for row in changed {
        stored.bundles.insert(row.header.name.clone(), row);
    }
    stored.deployed.record(&facts);
    Ok(())
}

/// Returns the facts that give the schema owner every family `stored` holds a bundle of, and
/// mark the file as recording family owners, when it does not yet.
fn legacy(stored: &Stored) -> Vec<Fact> {
    if stored.deployed.marked {
        return Vec::new();
    }
    let families: BTreeSet<&String> = stored.bundles.values().map(|r| &r.header.family).collect();
    std::iter::once(Fact::Deployed)
        .chain(
            families
                .into_iter()
                .map(|f| Fact::Family(f.clone(), OWNER_ID)),
        )
        .collect()
}

/// Refuse the registration of `header` with `calls`: a retired bundle's name with `Retired`,
/// and a name recorded with other content.
pub(super) fn admitted(stored: &Stored, header: &Header, calls: &[CallRow]) -> Result<(), Error> {
    let Some(row) = stored.bundles.get(&header.name) else {
        return Ok(());
    };
    if let State::Retired(_) = row.state {
        return Err(Error::rejected(RejectReason::Retired {
            bundle: header.name.clone(),
        }));
    }
    let before = stored.calls_of(row.id);
    let same_calls = before.is_empty() || before.iter().copied().eq(calls.iter());
    if row.header != *header || !same_calls {
        return Err(Error::invalid(format!(
            "bundle {} is registered with different content; a new release is a bundle of a new name",
            header.name
        )));
    }
    Ok(())
}

/// Returns the refusal of registering `header` at `now`, under the file settings `s`, because it
/// would read `ctor`, a constructor enabled since its pin: `BundleExpired` for a bundle whose
/// lease ran out, else `BundleStale`.
pub(super) fn stale(
    stored: &Stored,
    header: &Header,
    now: i64,
    s: &Settings,
    ctor: String,
) -> Error {
    let bundle = header.name.clone();
    let expired = stored
        .bundles
        .get(&bundle)
        .is_some_and(|r| r.state == State::Active && r.expired(now, s));
    let reason = if expired {
        RejectReason::BundleExpired {
            bundle,
            query: String::new(),
        }
    } else {
        RejectReason::BundleStale { bundle, ctor }
    };
    Error::rejected(reason)
}

/// Returns whether the bundle of `header` is registered again after it was superseded, or after
/// its lease ran out at `now` under the file settings `s`.
pub(super) fn lapsed(stored: &Stored, header: &Header, now: i64, s: &Settings) -> bool {
    stored
        .bundles
        .get(&header.name)
        .is_some_and(|r| match r.state {
            State::Superseded(_) => true,
            State::Active => r.expired(now, s),
            State::Retired(_) => false,
        })
}

/// Make each bundle of `regs`, registered by an open, live: start a new lease for one that was
/// not live, renew a lease that is due, store the calls of one never registered, record the
/// successful registration of one without, and give the schema owner each family with no owner,
/// as one durable transaction, at `now` under the file settings `s`. Returns the row of every
/// bundle and the deployment facts.
pub(super) fn register<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    mut stored: Stored,
    regs: &[Passed<'_>],
    now: i64,
    s: &Settings,
) -> Result<Stored, OpenError> {
    let mut writes = Writes::default();
    let mut next_call = stored.next_call();
    let mut families = stored.deployed.families.clone();
    for p in regs {
        let (h, calls) = (p.header, &p.calls);
        let row = stored
            .bundles
            .get(&h.name)
            .unwrap_or_else(|| panic!("bundle {} is recorded before it registers", h.name));
        if !row.live(now, s) || row.renewal_due(now, s) {
            let mut next = row.clone();
            next.state = State::Active;
            next.renewed = now;
            writes.bundles.push((Some(row.clone()), next));
        }
        if stored.calls_of(row.id).is_empty() {
            for c in calls {
                writes.calls.push((next_call, row.id, c.clone()));
                next_call += 1;
            }
        }
        if !stored.deployed.registered.contains_key(&row.id) {
            let mark = Mark {
                role: OWNER_ID,
                origin: Origin::Open,
                text: p.text.to_owned(),
            };
            writes.facts.push(Fact::Registered(row.id, mark));
        }
        if !families.contains_key(&h.family) {
            families.insert(h.family.clone(), OWNER_ID);
            writes.facts.push(Fact::Family(h.family.clone(), OWNER_ID));
        }
    }
    let changed: Vec<Row> = writes.bundles.iter().map(|(_, r)| r.clone()).collect();
    let facts = writes.facts.clone();
    let added = writes.calls.clone();
    writes.commit(catalog, pager, txns)?;
    for row in changed {
        stored.bundles.insert(row.header.name.clone(), row);
    }
    for (id, bundle, c) in added {
        stored.calls.entry(bundle).or_default().push((id, c));
    }
    stored.deployed.record(&facts);
    Ok(stored)
}

/// The writes of registering a bundle through a role session: its row, the row it had, the
/// calls it adds with their row ids, and the deployment facts.
pub(super) struct Claim {
    pub(super) old: Option<Row>,
    pub(super) row: Row,
    pub(super) calls: Vec<(i64, CallRow)>,
    pub(super) facts: Vec<Fact>,
}

/// Returns the writes registering `p` over `stored` as role `owner` of id `role`, at `now` under
/// the file settings `s`. A bundle recorded with no successful registration becomes the role's;
/// one registered before keeps its owner. The first successful registration of a family gives
/// the role the family.
pub(super) fn claim(
    stored: &Stored,
    p: &Passed<'_>,
    owner: &str,
    role: i64,
    now: i64,
    s: &Settings,
) -> Claim {
    let old = stored.bundles.get(&p.header.name).cloned();
    let marked = old
        .as_ref()
        .and_then(|r| stored.deployed.registered.get(&r.id));
    let mut row = old.clone().unwrap_or_else(|| Row {
        id: stored.next_bundle(),
        header: p.header.clone(),
        owner: owner.to_owned(),
        renewed: now,
        state: State::Active,
    });
    if marked.is_none() {
        row.owner = owner.to_owned();
    }
    if !row.live(now, s) || row.renewal_due(now, s) {
        row.state = State::Active;
        row.renewed = now;
    }
    let calls = if stored.calls_of(row.id).is_empty() {
        let next = stored.next_call();
        (next..).zip(p.calls.iter().cloned()).collect()
    } else {
        Vec::new()
    };
    let mut facts = Vec::new();
    if marked.is_none() {
        let mark = Mark {
            role,
            origin: Origin::Command,
            text: p.text.to_owned(),
        };
        facts.push(Fact::Registered(row.id, mark));
    }
    if !stored.deployed.families.contains_key(&p.header.family) {
        facts.push(Fact::Family(p.header.family.clone(), role));
    }
    Claim {
        old,
        row,
        calls,
        facts,
    }
}

impl Claim {
    /// Pass each row the claim deletes, with `false`, and inserts, with `true`, to `write` with
    /// its relation, over the engine's `schema`.
    pub(super) fn rows(&self, schema: &Schema, write: &mut impl FnMut(RelId, bool, Val)) {
        let t = Tables::of(schema);
        if self.old.as_ref() != Some(&self.row) || !self.facts.is_empty() {
            if let Some(old) = &self.old {
                write(t.bundles, false, t.bundle_value(old));
            }
            write(t.bundles, true, t.bundle_value(&self.row));
        }
        for (id, c) in &self.calls {
            write(t.registered, true, t.call_value(*id, self.row.id, c));
        }
    }

    /// Hold the claim's rows in `stored`, once they commit.
    pub(super) fn record(&self, stored: &mut Stored) {
        stored
            .bundles
            .insert(self.row.header.name.clone(), self.row.clone());
        if !self.calls.is_empty() {
            stored.calls.insert(self.row.id, self.calls.clone());
        }
        stored.deployed.record(&self.facts);
    }
}

impl<R: Runtime> Engine<R> {
    /// Refuse a bind of bundle `name` when it is retired, superseded or expired, and renew its
    /// lease when a renewal is due. A name that is no bundle's is not refused.
    pub(super) fn bind(&self, name: &str) -> Result<(), Error> {
        if self.admit(name, "")? {
            self.renew(name);
        }
        Ok(())
    }

    /// Refuse a bind or call of bundle `name`, the call's query `query`, when the bundle is
    /// retired, superseded or expired. Returns whether a renewal of its lease is due. A name
    /// that is no bundle's is not refused.
    pub(super) fn admit(&self, name: &str, query: &str) -> Result<bool, Error> {
        let now = self.clock();
        let s = *self.settings.lock(&self.rt);
        let deployed = self.deployed.lock(&self.rt);
        let Some(row) = deployed.get(name) else {
            return Ok(false);
        };
        match row.refusal(now, &s, query) {
            Some(reason) => Err(Error::rejected(reason)),
            None => Ok(row.renewal_due(now, &s)),
        }
    }

    /// Renew the lease of bundle `name` when a renewal is due and no other write of a bundle's
    /// row is under way.
    pub(super) fn renew(&self, name: &str) {
        let Some(_writer) = self.bundle_writer.try_lock() else {
            return;
        };
        let now = self.clock();
        let s = *self.settings.lock(&self.rt);
        let Some(row) = self.deployed.lock(&self.rt).get(name).cloned() else {
            return;
        };
        if row.refusal(now, &s, "").is_some() || !row.renewal_due(now, &s) {
            return;
        }
        let mut next = row.clone();
        next.renewed = now;
        // A renewal that fails leaves the lease as it was, for a later call to renew.
        if self.replace_bundle(&row, &next).is_ok() {
            self.deployed.lock(&self.rt).insert(name.to_owned(), next);
        }
    }

    /// Run `c` when it is a command on deployment state, `retire bundle` or `supersede
    /// bundle`, which no transaction holds. Returns its rows, `None` for any other command.
    pub(super) fn deployment_command(&self, c: &Command) -> Option<Result<DynRows, Error>> {
        let name = |b: &BundleName| format!("{}-{}", b.family, b.version);
        match c {
            Command::RetireBundle(b) => Some(self.retire(&name(b))),
            Command::SupersedeBundle(b, by) => Some(self.supersede(&name(b), &name(by))),
            _ => None,
        }
    }

    /// Retire bundle `name`. Returns its row.
    pub(super) fn retire(&self, name: &str) -> Result<DynRows, Error> {
        let _writer = self.bundle_writer.lock(&self.rt);
        let now = self.clock();
        let row = known(&self.deployed.lock(&self.rt), name)?.clone();
        let mut next = row.clone();
        if !matches!(row.state, State::Retired(_)) {
            next.state = State::Retired(now);
            self.replace_bundle(&row, &next)?;
        }
        let out = self.bundle_rows(&next);
        self.deployed.lock(&self.rt).insert(name.to_owned(), next);
        Ok(out)
    }

    /// Supersede bundle `name` by `by`, a live bundle of its family. Returns the row of `name`.
    pub(super) fn supersede(&self, name: &str, by: &str) -> Result<DynRows, Error> {
        let _writer = self.bundle_writer.lock(&self.rt);
        let now = self.clock();
        let s = *self.settings.lock(&self.rt);
        let (row, successor) = {
            let deployed = self.deployed.lock(&self.rt);
            (
                known(&deployed, name)?.clone(),
                known(&deployed, by)?.clone(),
            )
        };
        if name == by || successor.header.family != row.header.family || !successor.live(now, &s) {
            return Err(Error::invalid(format!(
                "bundle {name} is superseded only by another live bundle of family {}",
                row.header.family
            )));
        }
        if let State::Retired(_) = row.state {
            return Err(Error::invalid(format!("bundle {name} is retired")));
        }
        let mut next = row.clone();
        if row.state == State::Active {
            next.state = State::Superseded(now);
            self.replace_bundle(&row, &next)?;
        }
        let out = self.bundle_rows(&next);
        self.deployed.lock(&self.rt).insert(name.to_owned(), next);
        Ok(out)
    }

    /// Replace the row `old` of `catalog::bundles` by `new`, in a transaction of its own.
    fn replace_bundle(&self, old: &Row, new: &Row) -> Result<(), Error> {
        let t = Tables::of(&self.schema);
        self.swap_row(
            &self.schema,
            t.bundles,
            super::command::Old::Row(t.bundle_value(old)),
            t.bundle_value(new),
        )
        .map_err(|e| self.write_error(super::TxnId(0), e))
    }

    pub(super) fn bundle_rows(&self, row: &Row) -> DynRows {
        let t = Tables::of(&self.schema);
        let elem = Ty::Data {
            def: self.schema.relation(t.bundles).elem_def,
            args: Vec::new(),
            ctors: None,
        };
        rows(&self.schema, elem, vec![t.bundle_value(row)])
    }
}

/// Returns the row of bundle `name`, or `UnknownBundle`.
fn known<'d>(deployed: &'d BTreeMap<String, Row>, name: &str) -> Result<&'d Row, Error> {
    deployed.get(name).ok_or_else(|| {
        Error::rejected(RejectReason::UnknownBundle {
            bundle: name.to_owned(),
        })
    })
}
