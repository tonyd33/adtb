//! Bundles: the header of a bundle file, the rows of `catalog::bundles` and `catalog::registered`
//! that hold their deployment state, leases, retirement and supersession.

use std::collections::BTreeMap;

use adtb_lang::ast::{self, BundleHeader, BundleName, ExprKind, HeaderClause, Lit};
use adtb_lang::check::Command;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::{RelId, Ty};

use crate::catalog::{Catalog, Rewrite, layout};
use crate::exec::{self, Val};
use crate::key::Value;
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::error::{Error, RejectReason};
use super::settings::Settings;
use super::text::{DynRows, rows};
use super::{Engine, OpenError, corrupt};

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
    /// The constructor of `catalog::Priority` the header names.
    pub(super) priority: String,
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
            priority: "Regular".into(),
        };
        for clause in &h.clauses {
            match clause {
                HeaderClause::Lease(e) => header.lease = Some(lease(e)?),
                HeaderClause::Priority(p) => header.priority = priority(&p.text)?,
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

/// Returns the constructor of `catalog::Priority` the header word `p` names.
fn priority(p: &str) -> Result<String, Error> {
    ["AdHoc", "Background", "Regular"]
        .into_iter()
        .find(|c| c.eq_ignore_ascii_case(p))
        .map(str::to_owned)
        .ok_or_else(|| Error::invalid(format!("no priority `{p}`")))
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
#[derive(Debug, Default)]
pub(super) struct Stored {
    pub(super) bundles: BTreeMap<String, Row>,
    /// The registered calls of each bundle, by bundle id, with their row ids.
    calls: BTreeMap<i64, Vec<(i64, CallRow)>>,
}

impl Stored {
    fn next_bundle(&self) -> i64 {
        self.bundles.values().map(|r| r.id).max().unwrap_or(0) + 1
    }

    fn next_call(&self) -> i64 {
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
                Val::ctor(self.ctor("Priority", &h.priority), Vec::new()),
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
                Some(Val::Ctor(c, _)) => self.ctor_name("Priority", *c),
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
    Ok(out)
}

/// The rows one deployment write puts in place: each bundle row with the row it replaces, and
/// the calls it adds, each with its row id and its bundle's id.
#[derive(Default)]
struct Writes {
    bundles: Vec<(Option<Row>, Row)>,
    calls: Vec<(i64, i64, CallRow)>,
}

impl Writes {
    /// Write these rows as one durable transaction.
    fn commit<R: Runtime>(
        self,
        catalog: &Catalog,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<(), OpenError> {
        if self.bundles.is_empty() && self.calls.is_empty() {
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
            .deploy(pager, txns, &[bundles, calls])
            .map_err(corrupt)
    }
}

/// Record each bundle of `headers` that is not recorded yet, and supersede every active bundle
/// they do not name, as one durable transaction.
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
    for row in stored.bundles.values() {
        if row.state == State::Active && !headers.iter().any(|h| h.name == row.header.name) {
            let mut next = row.clone();
            next.state = State::Superseded(now);
            writes.bundles.push((Some(row.clone()), next));
        }
    }
    let changed: Vec<Row> = writes.bundles.iter().map(|(_, r)| r.clone()).collect();
    writes.commit(catalog, pager, txns)?;
    for row in changed {
        stored.bundles.insert(row.header.name.clone(), row);
    }
    Ok(())
}

/// Refuses the registration of `header` with `calls`: a retired bundle's name, and a name
/// recorded with other content.
pub(super) fn admitted(stored: &Stored, header: &Header, calls: &[CallRow]) -> Result<(), Error> {
    let Some(row) = stored.bundles.get(&header.name) else {
        return Ok(());
    };
    if let State::Retired(_) = row.state {
        return Err(Error::invalid(format!(
            "bundle {} is retired, and a retired bundle's name is never registered again",
            header.name
        )));
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

/// Make each bundle of `regs`, a header and its calls that passed every check, live: start a new
/// lease for one that was not live, renew a lease that is due, and store the calls of one never
/// registered, as one durable transaction, at `now` under the file settings `s`. Returns the
/// row of every bundle.
pub(super) fn register<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    mut stored: Stored,
    regs: &[(&Header, Vec<CallRow>)],
    now: i64,
    s: &Settings,
) -> Result<BTreeMap<String, Row>, OpenError> {
    let mut writes = Writes::default();
    let mut next_call = stored.next_call();
    for (h, calls) in regs {
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
    }
    let changed: Vec<Row> = writes.bundles.iter().map(|(_, r)| r.clone()).collect();
    writes.commit(catalog, pager, txns)?;
    for row in changed {
        stored.bundles.insert(row.header.name.clone(), row);
    }
    Ok(stored.bundles)
}

impl<R: Runtime> Engine<R> {
    /// Refuse a bind or call of bundle `name`, the call's query `query`, when the bundle is
    /// retired, superseded or expired, and renew its lease when a renewal is due. A name that
    /// is no bundle's is not refused.
    pub(super) fn bind(&self, name: &str, query: &str) -> Result<(), Error> {
        let now = self.clock();
        let s = *self.settings.lock(&self.rt);
        let mut deployed = self.deployed.lock(&self.rt);
        let Some(row) = deployed.get(name) else {
            return Ok(());
        };
        if let Some(reason) = row.refusal(now, &s, query) {
            return Err(Error::rejected(reason));
        }
        if row.renewal_due(now, &s) {
            let mut next = row.clone();
            next.renewed = now;
            self.replace_bundle(row, &next)?;
            deployed.insert(name.to_owned(), next);
        }
        Ok(())
    }

    /// Runs `c` when it is a command on deployment state, `retire bundle` or `supersede
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
        let now = self.clock();
        let mut deployed = self.deployed.lock(&self.rt);
        let row = known(&deployed, name)?;
        let mut next = row.clone();
        if !matches!(row.state, State::Retired(_)) {
            next.state = State::Retired(now);
            self.replace_bundle(row, &next)?;
        }
        let out = self.bundle_rows(&next);
        deployed.insert(name.to_owned(), next);
        Ok(out)
    }

    /// Supersede bundle `name` by `by`, a live bundle of its family. Returns the row of `name`.
    pub(super) fn supersede(&self, name: &str, by: &str) -> Result<DynRows, Error> {
        let now = self.clock();
        let s = *self.settings.lock(&self.rt);
        let mut deployed = self.deployed.lock(&self.rt);
        let row = known(&deployed, name)?;
        let successor = known(&deployed, by)?;
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
            self.replace_bundle(row, &next)?;
        }
        let out = self.bundle_rows(&next);
        deployed.insert(name.to_owned(), next);
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

    fn bundle_rows(&self, row: &Row) -> DynRows {
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
