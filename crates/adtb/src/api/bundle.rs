//! Bundles: the header of a bundle file, the rows of `catalog::bundles` and `catalog::registered`
//! that hold their deployment state, leases, retirement and supersession.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use adtb_lang::ast::{self, BundleHeader, BundleName, ExprKind, HeaderClause, Lit};
use adtb_lang::check::Command;
use adtb_lang::ids::{CatCtorId, engine_ctor as k};
use adtb_lang::parse::reads_as_bundle_name;
use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty};
use adtb_sys::lock::{Lock, Ranked, RankedGuard};

use crate::catalog::codec::{
    self, Codec, Ctor, Decode, Field, Fields, Kind, Relation, Sum, field, nullary, payload,
    relation, sum,
};
use crate::catalog::{Catalog, Deployed, Fact, Mark, Origin, Rewrite, Rot, layout};
use crate::exec::{self, Val};
use crate::key::{Scalar, Value};
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;
use crate::write;

use super::error::{Error, RejectReason};
use super::settings::Settings;
use super::text::{DynRows, Prepared, rows};
use super::{Engine, OWNER_ID, OWNER_ROLE, OpenError, corrupt};

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

/// Each priority class with the word that names it.
const PRIORITIES: [(&str, exec::Priority); 3] = [
    ("AdHoc", exec::Priority::AdHoc),
    ("Background", exec::Priority::Background),
    ("Regular", exec::Priority::Regular),
];

/// A value the engine built as a row of `catalog::bundles` or `catalog::registered` that is no
/// constructor's value.
#[derive(Debug)]
pub(super) enum BadRow {
    NotARow(Val),
}

impl fmt::Display for BadRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BadRow::NotARow(v) => write!(f, "a catalog row is {v:?}"),
        }
    }
}

/// Returns the priority class the word `p` names, in any case.
fn priority(p: &str) -> Option<exec::Priority> {
    PRIORITIES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(p))
        .map(|(_, c)| *c)
}

/// Returns the fixed id of the constructor of `catalog::Priority` that names class `c`.
fn priority_ctor(c: exec::Priority) -> CatCtorId {
    match c {
        exec::Priority::AdHoc => k::PRIORITY_AD_HOC,
        exec::Priority::Background => k::PRIORITY_BACKGROUND,
        exec::Priority::Regular => k::PRIORITY_REGULAR,
    }
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
pub(crate) struct Row {
    pub(super) id: i64,
    pub(super) header: Header,
    pub(super) owner: String,
    /// The last renewal of the lease, in microseconds since the epoch.
    pub(super) renewed: i64,
    pub(super) state: State,
    /// Whether the lease stays expired whatever the settings, until a registration renews it.
    pub(super) latched: bool,
}

impl Row {
    pub(super) fn retired(&self) -> bool {
        matches!(self.state, State::Retired(_))
    }

    /// Returns whether the lease is latched as expired or has run out at `now` under the file
    /// settings `s`, with the primary's grace.
    fn expired(&self, now: i64, s: &Settings) -> bool {
        self.latched || now >= self.renewed + self.header.lease(s) + s.grace
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

    /// Returns when the lease runs out under the file settings `s`, in microseconds since the
    /// epoch, before the primary's grace.
    pub(super) fn lease_until(&self, s: &Settings) -> i64 {
        self.renewed + self.header.lease(s)
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

/// What a registered call is: a query, a transaction or an imported function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Callable {
    Query,
    Txn,
    Fn,
}

impl Callable {
    /// Returns the constructor of `catalog::Registered` a call of this kind is a row of.
    fn ctor(self) -> CatCtorId {
        match self {
            Callable::Query => k::REGISTERED_QUERY,
            Callable::Txn => k::REGISTERED_TXN,
            Callable::Fn => k::REGISTERED_IMPORTED_FN,
        }
    }
}

/// One registered call of a bundle, as a row of `catalog::registered` holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CallRow {
    pub(super) kind: Callable,
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

    /// Returns the name of each bundle live at `now` under the file settings `s` that a
    /// registration has succeeded for, which migrations check against.
    pub(super) fn readers(&self, now: i64, s: &Settings) -> BTreeSet<String> {
        self.bundles
            .values()
            .filter(|r| r.live(now, s) && self.deployed.registered.contains_key(&r.id))
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

    /// Returns the id of the role bundle row `row` runs as.
    pub(super) fn owner_of(&self, row: &Row) -> i64 {
        self.deployed
            .registered
            .get(&row.id)
            .map_or(OWNER_ID, |m| m.role)
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
    pub(super) fn calls_of(&self, id: i64) -> Vec<&CallRow> {
        self.calls
            .get(&id)
            .map(|cs| cs.iter().map(|(_, c)| c).collect())
            .unwrap_or_default()
    }
}

const BUNDLE: Fields = &[
    "id",
    "name",
    "family",
    "owner",
    "pins",
    "hash",
    "edition",
    "catalog",
    "hides_new",
    "lease",
    "renewed",
    "priority",
    "memory_share",
    "workers",
    "state",
];
const BUNDLE_ID: Field = field(BUNDLE, "id");
const BUNDLE_NAME: Field = field(BUNDLE, "name");
const FAMILY: Field = field(BUNDLE, "family");
const OWNER: Field = field(BUNDLE, "owner");
const PINS: Field = field(BUNDLE, "pins");
const HASH: Field = field(BUNDLE, "hash");
const EDITION_F: Field = field(BUNDLE, "edition");
const CATALOG: Field = field(BUNDLE, "catalog");
const HIDES_NEW: Field = field(BUNDLE, "hides_new");
const LEASE: Field = field(BUNDLE, "lease");
const RENEWED: Field = field(BUNDLE, "renewed");
const MEMORY_SHARE: Field = field(BUNDLE, "memory_share");
const WORKERS: Field = field(BUNDLE, "workers");
const PRIORITY: Sum = sum(
    field(BUNDLE, "priority"),
    &[
        nullary(k::PRIORITY_AD_HOC),
        nullary(k::PRIORITY_BACKGROUND),
        nullary(k::PRIORITY_REGULAR),
    ],
);
const AT: Fields = &["at"];
const AT_KIND: &[Kind] = &[Kind::Scalar(Scalar::Instant)];
const STATE: Sum = sum(
    field(BUNDLE, "state"),
    &[
        nullary(k::BUNDLE_ACTIVE),
        payload(k::BUNDLE_SUPERSEDED, AT, AT_KIND),
        payload(k::BUNDLE_RETIRED, AT, AT_KIND),
    ],
);

impl Relation for relation::Bundles {
    type Row = Row;
}

impl Decode for Row {
    type Relation = relation::Bundles;
    const CTORS: &'static [Ctor<Row>] =
        &[Ctor::new(k::BUNDLE, BUNDLE, bundle_row).with_sums(&[PRIORITY, STATE])];
}

/// Returns the bundle row `r` holds, its lease not latched.
fn bundle_row(r: &codec::Row<'_>) -> Result<Row, Rot> {
    let (name, family) = (r.text(BUNDLE_NAME), r.text(FAMILY));
    if !reads_as_bundle_name(name) {
        return Err(r.refused(format!(
            "names `{name}`, not a bundle name of a family, a hyphen and a version"
        )));
    }
    let (named, version) = name
        .rsplit_once('-')
        .expect("a bundle name holds a hyphen before its version");
    if named != family {
        return Err(r.refused(format!(
            "names bundle `{name}` under family `{family}`, not `{named}`"
        )));
    }
    let catalog = match r.optional_int(CATALOG) {
        Some(c) => Some(u64::try_from(c).map_err(|_| r.bad(CATALOG))?),
        None => None,
    };
    let priority = match r.ctor(PRIORITY).0 {
        k::PRIORITY_AD_HOC => exec::Priority::AdHoc,
        k::PRIORITY_BACKGROUND => exec::Priority::Background,
        k::PRIORITY_REGULAR => exec::Priority::Regular,
        other => codec::unlisted(other),
    };
    let state = match r.ctor(STATE) {
        (k::BUNDLE_ACTIVE, _) => State::Active,
        (k::BUNDLE_SUPERSEDED, at) => State::Superseded(codec::payload_instant(at)),
        (k::BUNDLE_RETIRED, at) => State::Retired(codec::payload_instant(at)),
        (other, _) => codec::unlisted(other),
    };
    Ok(Row {
        id: r.int(BUNDLE_ID),
        header: Header {
            name: name.to_owned(),
            family: family.to_owned(),
            version: version.to_owned(),
            pins: u64::try_from(r.int(PINS)).map_err(|_| r.bad(PINS))?,
            hash: r.bytes(HASH).iter().map(|x| format!("{x:02x}")).collect(),
            catalog,
            edition: r.text(EDITION_F).to_owned(),
            lease: r.optional_duration(LEASE),
            hides_new: r.flag(HIDES_NEW),
            priority,
        },
        owner: r.text(OWNER).to_owned(),
        renewed: r.instant(RENEWED),
        state,
        latched: false,
    })
}

/// A row of `catalog::registered`: a call with its row id and its bundle's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Registered {
    id: i64,
    bundle: i64,
    call: CallRow,
}

impl Registered {
    /// Returns the id of the call's bundle.
    pub(super) fn bundle(&self) -> i64 {
        self.bundle
    }
}

const CALL: Fields = &[
    "id",
    "bundle",
    "name",
    "ordinal",
    "signature",
    "text",
    "limit",
];
const CALL_ID: Field = field(CALL, "id");
const CALL_BUNDLE: Field = field(CALL, "bundle");
const CALL_NAME: Field = field(CALL, "name");
const ORDINAL: Field = field(CALL, "ordinal");
const SIGNATURE: Field = field(CALL, "signature");
const TEXT: Field = field(CALL, "text");
const CALL_LIMIT: Field = field(CALL, "limit");

impl Relation for relation::Registered {
    type Row = Registered;
}

impl Decode for Registered {
    type Relation = relation::Registered;
    const CTORS: &'static [Ctor<Registered>] = &[
        Ctor::new(k::REGISTERED_QUERY, CALL, |r| {
            registered(r, Callable::Query)
        }),
        Ctor::new(k::REGISTERED_TXN, CALL, |r| registered(r, Callable::Txn)),
        Ctor::new(k::REGISTERED_IMPORTED_FN, CALL, |r| {
            registered(r, Callable::Fn)
        }),
    ];
}

/// Returns the registered call of kind `kind` that `r` holds.
fn registered(r: &codec::Row<'_>, kind: Callable) -> Result<Registered, Rot> {
    Ok(Registered {
        id: r.int(CALL_ID),
        bundle: r.int(CALL_BUNDLE),
        call: CallRow {
            kind,
            name: r.text(CALL_NAME).to_owned(),
            ordinal: r.int32(ORDINAL),
            signature: r.text(SIGNATURE).to_owned(),
            text: r.text(TEXT).to_owned(),
        },
    })
}

/// The relations the deployment rows live in, with their codecs.
struct Tables<'s> {
    schema: &'s Schema,
    bundle_codec: Codec<Row>,
    call_codec: Codec<Registered>,
    bundles: RelId,
    registered: RelId,
}

impl<'s> Tables<'s> {
    fn of(schema: &'s Schema) -> Tables<'s> {
        let bundle_codec = Codec::<Row>::new(schema);
        let call_codec = Codec::<Registered>::new(schema);
        Tables {
            schema,
            bundles: bundle_codec.relation(),
            registered: call_codec.relation(),
            bundle_codec,
            call_codec,
        }
    }

    /// Returns the value of `row`.
    fn bundle_value(&self, row: &Row) -> Val {
        let h = &row.header;
        let codec = &self.bundle_codec;
        let state = match row.state {
            State::Active => Val::ctor(codec.tag(STATE, k::BUNDLE_ACTIVE), Vec::new()),
            State::Superseded(at) => Val::ctor(
                codec.tag(STATE, k::BUNDLE_SUPERSEDED),
                vec![Val::Instant(at)],
            ),
            State::Retired(at) => {
                Val::ctor(codec.tag(STATE, k::BUNDLE_RETIRED), vec![Val::Instant(at)])
            }
        };
        let priority = codec.tag(PRIORITY, priority_ctor(h.priority));
        let (c, fields) = codec.encode(
            k::BUNDLE,
            [
                (BUNDLE_ID, Val::Int(row.id)),
                (BUNDLE_NAME, Val::text(&h.name)),
                (FAMILY, Val::text(&h.family)),
                (OWNER, Val::text(&row.owner)),
                (PINS, Val::Int(int(h.pins))),
                (HASH, Val::Bytes(unhex(&h.hash).into())),
                (EDITION_F, Val::text(&h.edition)),
                (
                    CATALOG,
                    h.catalog
                        .map_or_else(Val::none, |c| Val::some(Val::Int(int(c)))),
                ),
                (HIDES_NEW, Val::bool(h.hides_new)),
                (
                    LEASE,
                    h.lease
                        .map_or_else(Val::none, |l| Val::some(Val::Duration(l))),
                ),
                (RENEWED, Val::Instant(row.renewed)),
                (PRIORITY.field(), Val::ctor(priority, Vec::new())),
                (MEMORY_SHARE, Val::none()),
                (WORKERS, Val::none()),
                (STATE.field(), state),
            ],
        );
        Val::ctor(c, fields)
    }

    /// Returns the value of call `c`, row `id` of bundle `bundle`.
    fn call_value(&self, id: i64, bundle: i64, c: &CallRow) -> Val {
        let (kind, fields) = self.call_codec.encode(
            c.kind.ctor(),
            [
                (CALL_ID, Val::Int(id)),
                (CALL_BUNDLE, Val::Int(bundle)),
                (CALL_NAME, Val::text(&c.name)),
                (ORDINAL, Val::Int(c.ordinal)),
                (SIGNATURE, Val::text(&c.signature)),
                (TEXT, Val::text(&c.text)),
                (CALL_LIMIT, Val::none()),
            ],
        );
        Val::ctor(kind, fields)
    }

    /// Returns the stored form of `v`, a row of `rel`.
    fn stored(&self, rel: RelId, v: &Val) -> Result<(u32, Vec<Value>), OpenError> {
        let Val::Ctor(c, fs) = v else {
            return Err(corrupt(BadRow::NotARow(v.clone())));
        };
        let fields = layout::row(self.schema, rel, *c)
            .iter()
            .zip(fs.iter())
            .map(|((_, t), v)| exec::to_stored(&layout::field_type(self.schema, t), v))
            .collect::<Result<Vec<_>, _>>()
            .map_err(corrupt)?;
        Ok((*c, fields))
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
    let schema = catalog.schema();
    let mut out = Stored::default();
    let bundles = catalog.committed(pager, txns, &Codec::<Row>::new(schema));
    for row in bundles.map_err(corrupt)? {
        out.bundles.insert(row.header.name.clone(), row);
    }
    let calls = catalog.committed(pager, txns, &Codec::<Registered>::new(schema));
    for r in calls.map_err(corrupt)? {
        out.calls.entry(r.bundle).or_default().push((r.id, r.call));
    }
    for calls in out.calls.values_mut() {
        calls.sort_by_key(|(_, c)| c.ordinal);
    }
    out.deployed = catalog.deployed(pager, txns).map_err(corrupt)?;
    for row in out.bundles.values_mut() {
        row.latched = out.deployed.expired.get(&row.id) == Some(&row.renewed);
    }
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
            .map(drop)
            .map_err(corrupt)
    }
}

/// Record each bundle of `headers` that is not recorded yet, and supersede every active bundle
/// an open registered that they do not name, as one durable transaction. On a file without
/// family owners yet, give the schema owner every family it holds a bundle of.
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
                owner: OWNER_ROLE.to_owned(),
                renewed: now,
                state: State::Active,
                latched: false,
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
    let family = &header.family;
    if !stored.deployed.families.contains_key(family) && reads_as_bundle_name(family) {
        return Err(Error::invalid(format!(
            "family `{family}` of bundle {} reads as a bundle name",
            header.name
        )));
    }
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

/// Returns whether registering the bundle of `header` at `now`, under the file settings `s`, makes
/// it live: no earlier registration succeeded, or it lapsed since.
pub(super) fn going_live(stored: &Stored, header: &Header, now: i64, s: &Settings) -> bool {
    lapsed(stored, header, now, s)
        || stored
            .bundles
            .get(&header.name)
            .is_none_or(|r| !stored.deployed.registered.contains_key(&r.id))
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

/// Make each bundle of `regs`, given to an open, live as [`claim`] does for the schema owner, as
/// one durable transaction, at `now` under the file settings `s`. Returns the rows as they
/// then stand.
pub(super) fn register<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    mut stored: Stored,
    regs: &[Passed<'_>],
    now: i64,
    s: &Settings,
) -> Result<Stored, OpenError> {
    let owner = Registrant {
        id: OWNER_ID,
        name: OWNER_ROLE,
    };
    let mut claims = Vec::with_capacity(regs.len());
    for p in regs {
        assert!(
            stored.bundles.contains_key(&p.header.name),
            "bundle {} is recorded before it registers",
            p.header.name
        );
        let c = claim(&stored, p, owner, Origin::Open, now, s);
        c.record(&mut stored);
        claims.push(c);
    }
    let mut writes = Writes::default();
    for c in claims {
        if c.changed() {
            writes.bundles.push((c.old, c.row.clone()));
        }
        let bundle = c.row.id;
        writes
            .calls
            .extend(c.calls.into_iter().map(|(id, call)| (id, bundle, call)));
        writes.facts.extend(c.facts);
    }
    writes.commit(catalog, pager, txns)?;
    Ok(stored)
}

/// The role a bundle registers as: its id and name.
#[derive(Clone, Copy, Debug)]
pub(super) struct Registrant<'a> {
    pub(super) id: i64,
    pub(super) name: &'a str,
}

/// The writes of making a bundle live: its row, the row it had, the id of the role it runs as
/// and whether an earlier successful registration fixed it, its new calls with their row ids,
/// and the deployment facts.
pub(super) struct Claim {
    pub(super) old: Option<Row>,
    pub(super) row: Row,
    pub(super) owner: i64,
    pub(super) owned: bool,
    pub(super) calls: Vec<(i64, CallRow)>,
    pub(super) facts: Vec<Fact>,
}

/// Returns the writes registering `p` over `stored` as `by`, through `origin`, at `now` under
/// the file settings `s`.
///
/// Postconditions:
/// - A bundle with no successful registration becomes `by`'s; one registered before keeps its
///   owner.
/// - A bundle that is not live gets a new lease, and one whose renewal is due is renewed.
/// - The renewal of a latched bundle differs from the one its latch names.
/// - A bundle with no stored calls stores those of `p`.
/// - The first successful registration of a family gives `by` the family.
pub(super) fn claim(
    stored: &Stored,
    p: &Passed<'_>,
    by: Registrant<'_>,
    origin: Origin,
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
        owner: by.name.to_owned(),
        renewed: now,
        state: State::Active,
        latched: false,
    });
    if marked.is_none() {
        row.owner = by.name.to_owned();
    }
    if !row.live(now, s) || row.renewal_due(now, s) {
        row.renewed = if row.latched {
            now.max(row.renewed + 1)
        } else {
            now
        };
        row.state = State::Active;
        row.latched = false;
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
            role: by.id,
            origin,
            text: p.text.to_owned(),
        };
        facts.push(Fact::Registered(row.id, mark));
    }
    if !stored.deployed.families.contains_key(&p.header.family) {
        facts.push(Fact::Family(p.header.family.clone(), by.id));
    }
    Claim {
        old,
        row,
        owner: marked.map_or(by.id, |m| m.role),
        owned: marked.is_some(),
        calls,
        facts,
    }
}

impl Claim {
    fn changed(&self) -> bool {
        self.old.as_ref() != Some(&self.row)
    }

    /// Pass each row the claim deletes, with `false`, and inserts, with `true`, to `write` with
    /// its relation, over the engine's `schema`.
    pub(super) fn rows(&self, schema: &Schema, write: &mut impl FnMut(RelId, bool, Val)) {
        let t = Tables::of(schema);
        if self.changed() {
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

pub(super) fn bundle_name(b: &BundleName) -> String {
    format!("{}-{}", b.family, b.version)
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
        let deployed = self.deployed.read(&self.rt);
        let Some(row) = deployed.bundles.get(name) else {
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
        let Some(writer) = self.try_bundle_writer() else {
            return;
        };
        let now = self.clock();
        let s = *self.settings.lock(&self.rt);
        let Some(row) = self.deployed.read(&self.rt).bundles.get(name).cloned() else {
            return;
        };
        if row.refusal(now, &s, "").is_some() || !row.renewal_due(now, &s) {
            return;
        }
        let mut next = row.clone();
        next.renewed = now;
        // A renewal that fails leaves the lease as it was, for a later call to renew.
        if writer.replace_bundle(&row, &next).is_ok() {
            writer.put(next);
        }
    }

    /// Run `c` when it is a command on deployment state, `retire bundle` or `supersede
    /// bundle`, which no transaction holds. Returns its rows, `None` for any other command.
    pub(super) fn deployment_command(
        &self,
        c: &Command,
        p: &Prepared,
    ) -> Option<Result<DynRows, Error>> {
        match c {
            Command::RetireBundle(b) => Some(self.retire(&bundle_name(b), p)),
            Command::SupersedeBundle(b, by) => {
                Some(self.supersede(&bundle_name(b), &bundle_name(by)))
            }
            _ => None,
        }
    }

    /// Retire bundle `name` for the issuer of `p`, the row and its `Retire` audit row committing
    /// together. Returns its row.
    pub(super) fn retire(&self, name: &str, p: &Prepared) -> Result<DynRows, Error> {
        let writer = self.bundle_writer();
        let now = self.clock();
        let row = known(&self.deployed.read(&self.rt).bundles, name)?.clone();
        let mut next = row.clone();
        if !matches!(row.state, State::Retired(_)) {
            next.state = State::Retired(now);
            let t = Tables::of(&self.schema);
            let role = p.issuer.as_ref().map_or(OWNER_ROLE, |i| &i.role);
            writer.retire_row(
                t.bundles,
                (t.bundle_value(&row), t.bundle_value(&next)),
                (role, p.caller()),
                name,
            )?;
        }
        let out = self.bundle_rows(&next);
        writer.put(next);
        Ok(out)
    }

    /// Supersede bundle `name` by `by`, a live bundle of its family. Returns the row of `name`.
    pub(super) fn supersede(&self, name: &str, by: &str) -> Result<DynRows, Error> {
        let writer = self.bundle_writer();
        let now = self.clock();
        let settings = *self.settings.lock(&self.rt);
        let (row, successor) = {
            let deployed = self.deployed.read(&self.rt);
            (
                known(&deployed.bundles, name)?.clone(),
                deployed.bundles.get(by).cloned(),
            )
        };
        let replaces = |other: &Row| {
            name != by && other.header.family == row.header.family && other.live(now, &settings)
        };
        if !successor.as_ref().is_some_and(replaces) {
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
            writer.replace_bundle(&row, &next)?;
        }
        let out = self.bundle_rows(&next);
        writer.put(next);
        Ok(out)
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

/// A [`Stored`] shared by reads, and the lock of its writers.
pub(super) struct StoredLock {
    stored: Lock<Arc<Stored>>,
    /// Held across every write of a row of `catalog::bundles`, so that the row a writer reads
    /// from `stored` is the one its write replaces.
    writer: Ranked<()>,
}

/// A holder of the writer lock of an engine's [`StoredLock`], the one way to change what it
/// stores.
pub(super) struct BundleWriter<'a, R: Runtime> {
    engine: &'a Engine<R>,
    _held: RankedGuard<'a, ()>,
}

impl StoredLock {
    pub(super) fn new(stored: Stored) -> StoredLock {
        StoredLock {
            stored: Lock::new(Arc::new(stored)),
            writer: Ranked::new(crate::ranks::BUNDLE_WRITER, ()),
        }
    }

    /// Returns what the latest commit holds.
    pub(super) fn read<R: Runtime>(&self, rt: &R) -> Arc<Stored> {
        self.stored.lock(rt).clone()
    }

    /// Returns the writer lock, waiting while another holds it.
    pub(super) fn write<R: Runtime>(&self, rt: &R) -> RankedGuard<'_, ()> {
        self.writer.lock(rt)
    }
}

impl<R: Runtime> Engine<R> {
    /// Returns the writer of the deployed bundles, waiting while another holds its lock.
    pub(super) fn bundle_writer(&self) -> BundleWriter<'_, R> {
        BundleWriter {
            engine: self,
            _held: self.deployed.write(&self.rt),
        }
    }

    /// Returns the writer of the deployed bundles unless another holds its lock.
    pub(super) fn try_bundle_writer(&self) -> Option<BundleWriter<'_, R>> {
        Some(BundleWriter {
            engine: self,
            _held: self.deployed.writer.try_lock()?,
        })
    }
}

impl<'a, R: Runtime> BundleWriter<'a, R> {
    /// Returns the engine whose bundles this writes.
    pub(super) fn engine(&self) -> &'a Engine<R> {
        self.engine
    }

    /// Apply `f` to the stored rows, once the write it mirrors has committed.
    pub(super) fn change(&self, f: impl FnOnce(&mut Stored)) {
        let e = self.engine;
        f(Arc::make_mut(&mut e.deployed.stored.lock(&e.rt)));
    }

    /// Latch each active bundle expired under the current settings, in a transaction of its own,
    /// before a lease setting changes.
    pub(super) fn latch_expired(&self) -> Result<(), write::Error> {
        let e = self.engine;
        let now = e.clock();
        let s = *e.settings.lock(&e.rt);
        let due: Vec<Row> = e
            .deployed
            .read(&e.rt)
            .bundles
            .values()
            .filter(|r| r.state == State::Active && !r.latched && r.expired(now, &s))
            .cloned()
            .collect();
        if due.is_empty() {
            return Ok(());
        }
        let facts: Vec<Fact> = due.iter().map(|r| Fact::Expired(r.id, r.renewed)).collect();
        let tx = e.db.begin()?;
        let x = tx
            .xid()
            .expect("a write transaction of the write path has an id");
        if let Err(err) = e.deployment.insert(&e.pager, e.db.txns(), x, &facts) {
            return Err(e.db.abandoned(tx, err.into()));
        }
        e.db.commit(tx)?;
        self.change(|s| s.deployed.record(&facts));
        for mut row in due {
            row.latched = true;
            self.put(row);
        }
        Ok(())
    }

    /// Hold `row`, once it commits, as its bundle's row.
    fn put(&self, row: Row) {
        self.change(|s| {
            s.bundles.insert(row.header.name.clone(), row);
        });
    }

    /// Replace the row `old` of `catalog::bundles` by `new`, in a transaction of its own.
    fn replace_bundle(&self, old: &Row, new: &Row) -> Result<(), Error> {
        let e = self.engine;
        let t = Tables::of(&e.schema);
        e.swap_row(
            &e.schema,
            t.bundles,
            super::command::Old::Row(t.bundle_value(old)),
            t.bundle_value(new),
        )
        .map_err(|err| e.write_error(super::TxnId(0), err))
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
