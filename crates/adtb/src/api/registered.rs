//! Registered queries and transactions: checked and normalised when the database opens, called
//! by ordinal with arguments, each for the principal of its call.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};

use adtb_lang::ast::{BundleItem, FnDecl, Import, QName};
use adtb_lang::check::{Bundles, Program, check_adhoc, check_query_in, check_txn_in};
use adtb_lang::evolve::Source;
use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::norm::{self, NormalQuery};
use adtb_lang::parse::{Options, parse_adhoc, parse_bundle, parse_bundle_body};
use adtb_lang::resolve::{Visible, lookup_value};
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::signature;
use adtb_lang::types::Ty;

use crate::catalog::{self, Catalog};
use crate::exec::Val;
use crate::runtime::Runtime;

use super::arrow::{self, ArrowStream};
use super::bundle::{self, CallRow, EDITION, Header, Stored};
use super::error::{Error, RejectReason};
use super::reads::Reads;
use super::settings::Settings;
use super::text::{DynRows, Prepared, rows};
use super::txn::{AdHocTransaction, Committed, TxnCallOptions};
use super::{CallOptions, Database, Engine, OpenError};

/// The queries, transactions and imported schema functions one unit registers: a bundle file
/// with its header, or the imports and items of one without.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    name: String,
    text: String,
    header: Option<Header>,
}

impl Registration {
    /// Returns the registration `name` of `text`, the imports and items of a bundle file with no
    /// header, checked at the version open leaves the file at.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Registration {
        Registration {
            name: name.into(),
            text: text.into(),
            header: None,
        }
    }

    /// Returns the registration of the bundle file `text`, named by its header and checked at
    /// the version it pins.
    ///
    /// Fails with `Invalid` when `text` does not parse as a bundle file, and when its header
    /// states a clause this engine does not run.
    pub fn bundle(text: impl Into<String>) -> Result<Registration, Error> {
        let text = text.into();
        let file = parse_bundle(&text, Options::default())
            .ok()
            .map_err(|ds| Error::invalid(joined(ds.iter().map(|d| d.message.as_str()))))?;
        let header = Header::of(&file.header)?;
        Ok(Registration {
            name: header.name.clone(),
            text,
            header: Some(header),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn header(&self) -> Option<&Header> {
        self.header.as_ref()
    }

    /// Returns the imports and items of the registration's text.
    fn body(&self) -> Result<(Vec<Import>, Vec<BundleItem>), Error> {
        let failed = |ds: Vec<adtb_lang::span::Diag>| {
            Error::invalid(joined(ds.iter().map(|d| d.message.as_str())))
        };
        if self.header.is_some() {
            let file = parse_bundle(&self.text, Options::default())
                .ok()
                .map_err(failed)?;
            Ok((file.imports, file.items))
        } else {
            parse_bundle_body(&self.text, Options::default())
                .ok()
                .map_err(failed)
        }
    }
}

/// What a call of a registered query, transaction or function returned outside an interactive
/// transaction.
#[derive(Debug)]
pub enum Called {
    /// The elements of a set-valued result, else the one value.
    Rows(DynRows),
    /// The one result of a registered transaction, with its commit position.
    Committed(Committed<DynRows>),
}

/// What a call inside an interactive transaction returned.
#[derive(Debug)]
pub enum InTxn {
    Rows(DynRows),
    /// The one result of a registered transaction, committed with the transaction.
    Pending(DynRows),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Callable {
    Query,
    Txn,
    Fn,
}

impl Callable {
    /// Returns the constructor of `catalog::Registered` a call of this kind is a row of.
    fn ctor(self) -> &'static str {
        match self {
            Callable::Query => "Query",
            Callable::Txn => "Txn",
            Callable::Fn => "ImportedFn",
        }
    }
}

/// One callable of a registration, normalised at the schema version it was registered at.
#[derive(Debug)]
struct Entry {
    name: String,
    kind: Callable,
    terms: Arc<Terms>,
    body: Tm,
    /// The variable each argument binds, in parameter order; `None` for an argument the body
    /// does not read.
    params: Vec<Option<Var>>,
    /// The type of each declared parameter, in order.
    types: Vec<Ty>,
    /// The variable of the unit parameter a body with no declared parameter takes.
    unit: Option<Var>,
    writes: bool,
    retry: Option<u64>,
    /// Its row of `catalog::registered`, its ordinal from 1.
    row: CallRow,
}

/// A registration whose every callable checked.
#[derive(Debug)]
pub(super) struct Checked {
    name: String,
    /// The schema its callables run over: the version's, with the registration's functions.
    schema: Arc<Schema>,
    entries: Vec<Entry>,
    /// The header of a bundle file's registration.
    header: Option<Header>,
    /// Whether the registration is checked at the version the database is at, so its calls run.
    current: bool,
}

impl Checked {
    /// Returns the header and calls of a bundle's registration.
    pub(super) fn bundle(&self) -> Option<(&Header, Vec<CallRow>)> {
        let calls = self.entries.iter().map(|e| e.row.clone()).collect();
        self.header.as_ref().map(|h| (h, calls))
    }

    /// Record the queries and transactions of a bundle whose calls run in `bundles`.
    pub(super) fn explained(&self, bundles: &mut Bundles) {
        let Some(h) = self.header.as_ref().filter(|_| self.current) else {
            return;
        };
        for e in self.entries.iter().filter(|e| e.kind != Callable::Fn) {
            bundles.insert(&h.family, &h.version, &e.name, e.types.clone());
        }
    }
}

/// The schema a registration checks against and the program of its functions.
struct Over<'a> {
    source: &'a Source,
    schema: &'a Arc<Schema>,
    program: &'a Program,
}

/// A version a bundle pins: its source, schema and program.
struct Pin {
    source: Source,
    schema: Arc<Schema>,
    program: Program,
}

impl Pin {
    fn over(&self) -> Over<'_> {
        Over {
            source: &self.source,
            schema: &self.schema,
            program: &self.program,
        }
    }
}

/// Check and normalise every registration of `regs`, all or none: one with no header against
/// `program` of `schema`, the schema of `catalog`'s version, and a bundle's at the version it
/// pins, refusing one `stored` holds otherwise or that would read a constructor enabled since
/// its pin, at `now` under the file settings `s`.
pub(super) fn register(
    catalog: &Catalog,
    schema: &Arc<Schema>,
    program: &Program,
    regs: &[Registration],
    stored: &Stored,
    now: i64,
    s: &Settings,
) -> Result<Vec<Checked>, OpenError> {
    let over = Over {
        source: catalog.source(),
        schema,
        program,
    };
    let mut names = BTreeSet::new();
    let mut out = Vec::with_capacity(regs.len());
    for r in regs {
        let refused = |reason: Error| OpenError::RegistrationRefused {
            bundle: r.name.clone(),
            reason,
        };
        if !names.insert(r.name.as_str()) {
            return Err(refused(Error::invalid(format!(
                "`{}` is registered twice",
                r.name
            ))));
        }
        let Some(h) = &r.header else {
            out.push(checked(&over, r, true).map_err(refused)?);
            continue;
        };
        let at_pin = pinned(catalog, h).map_err(refused)?;
        let c = match &at_pin {
            Some(pin) => checked(&pin.over(), r, false),
            None => checked(&over, r, true),
        }
        .map_err(refused)?;
        let (_, calls) = c.bundle().expect("a bundle's registration has a header");
        bundle::admitted(stored, h, &calls).map_err(refused)?;
        if let Some(ctor) = stale(catalog, &c).map_err(refused)? {
            return Err(refused(bundle::stale(stored, h, now, s, ctor)));
        }
        out.push(c);
    }
    Ok(out)
}

/// Refuses an open whose bundles pin a version before the file's and would read a constructor
/// enabled since. A bundle that does not check at its pin is left for registration to refuse.
pub(super) fn downgrade(catalog: &Catalog, regs: &[Registration]) -> Result<(), OpenError> {
    let version = catalog.source().version;
    for r in regs {
        let Some(h) = r.header.as_ref().filter(|h| h.pins < version) else {
            continue;
        };
        let Ok(Some(pin)) = pinned(catalog, h) else {
            continue;
        };
        let Ok(c) = checked(&pin.over(), r, false) else {
            continue;
        };
        if let Ok(Some(ctor)) = stale(catalog, &c) {
            return Err(OpenError::DowngradeRefused {
                cause: format!(
                    "bundle {}, pinned at version {}, reads {ctor}, enabled since",
                    h.name, h.pins
                ),
            });
        }
    }
    Ok(())
}

/// Returns the version `h` pins, `None` for the version `catalog` is at. Refuses an edition the
/// engine does not implement, a catalog pin, and a pinned version the log does not hold or holds
/// with another hash.
fn pinned(catalog: &Catalog, h: &Header) -> Result<Option<Pin>, Error> {
    if h.edition != EDITION {
        return Err(Error::rejected(RejectReason::UnknownEdition {
            edition: h.edition.clone(),
        }));
    }
    if let Some(v) = h.catalog {
        return Err(Error::invalid(format!(
            "bundle {} pins catalog version {v}, and this engine's catalog has no versions",
            h.name
        )));
    }
    let now = catalog.source().version;
    let hashed = |schema: &Schema, source: &Source| {
        let hash: String = catalog::version_hash(schema, source)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if hash == h.hash {
            return Ok(());
        }
        Err(Error::invalid(format!(
            "bundle {} pins version {} with hash {}, and the log's version {} has hash {hash}",
            h.name, h.pins, h.hash, h.pins
        )))
    };
    if h.pins == now {
        return hashed(catalog.schema(), catalog.source()).map(|()| None);
    }
    let source = catalog.source_at(h.pins).ok_or_else(|| {
        Error::invalid(format!(
            "bundle {} pins version {}, which the log does not hold: it is at version {now}",
            h.name, h.pins
        ))
    })?;
    let (schema, program) = source
        .check()
        .map_err(|ds| Error::invalid(joined(ds.iter().map(|d| d.diag.message.as_str()))))?;
    hashed(&schema, &source)?;
    Ok(Some(Pin {
        source,
        schema: Arc::new(schema),
        program,
    }))
}

/// Returns the constructor the bundle `c` would read that is enabled now and absent at its pin.
fn stale(catalog: &Catalog, c: &Checked) -> Result<Option<String>, Error> {
    let Some(h) = &c.header else {
        return Ok(None);
    };
    let mut reads = Reads::default();
    for e in &c.entries {
        reads.add(&c.schema, &e.terms, e.body);
    }
    let reader = reads.reader(&c.schema, &h.name, h.pins, h.hides_new);
    catalog.stale(&reader).map_err(|e| match e {
        catalog::Error::Refused(why) | catalog::Error::Unaccepted(_, why) => {
            Error::invalid(why.join("\n"))
        }
        other => Error::rejected(RejectReason::Internal {
            what: format!("{other:?}"),
        }),
    })
}

/// Check and normalise registration `r` over `over`, the version the database is at when
/// `current`.
fn checked(over: &Over<'_>, r: &Registration, current: bool) -> Result<Checked, Error> {
    let (imports, items) = r.body()?;
    let vis = signature::visible(over.schema, &imports).map_err(Error::invalid)?;
    let fns: Vec<&FnDecl> = items
        .iter()
        .filter_map(|i| match i {
            BundleItem::Fn(f) => Some(f),
            _ => None,
        })
        .collect();
    let extended = signature::with_functions(over.source, over.schema, &vis, &fns, &r.text)
        .map_err(Error::invalid)?
        .map(|(s, p)| (Arc::new(s), p));
    let (schema, program) = match &extended {
        Some((s, p)) => (s, p),
        None => (over.schema, over.program),
    };
    let functions: String = fns
        .iter()
        .map(|f| format!("\n{}", spanned(&r.text, f.span)))
        .collect();
    let mut entries = Vec::new();
    for i in &imports {
        if let Import::Fn(f) = i {
            entries.push(imported(schema, program, &vis, f)?);
        }
    }
    for item in &items {
        let decl = |span, name: &adtb_lang::ast::Ident, result: &adtb_lang::ast::Type| {
            let signature = r
                .text
                .get(name.span.end as usize..result.span.end as usize)
                .unwrap_or_default()
                .trim()
                .to_owned();
            (signature, format!("{}{functions}", spanned(&r.text, span)))
        };
        match item {
            BundleItem::Query(q) => {
                let failed = |m: String| Error::invalid(format!("query `{}`: {m}", q.name.text));
                if q.limit.is_some() {
                    return Err(failed(
                        "a declared limit does not run in this engine yet".into(),
                    ));
                }
                let checked = check_query_in(schema, program, vis.clone(), q)
                    .map_err(|d| failed(d.message))?;
                let normal = norm::query(schema, program, &checked).map_err(no_normal)?;
                let (signature, text) = decl(q.span, &q.name, &q.result);
                let mut e = entry(&q.name.text, Callable::Query, q.params.len(), normal);
                e.row.signature = signature;
                e.row.text = text;
                entries.push(e);
            }
            BundleItem::Txn(t) => {
                let failed = |m: String| Error::invalid(format!("txn `{}`: {m}", t.name.text));
                if t.limit.is_some() {
                    return Err(failed(
                        "a declared limit does not run in this engine yet".into(),
                    ));
                }
                let checked =
                    check_txn_in(schema, program, vis.clone(), t).map_err(|d| failed(d.message))?;
                let normal = norm::txn(schema, program, &checked).map_err(no_normal)?;
                let (signature, text) = decl(t.span, &t.name, &t.result);
                let mut e = entry(&t.name.text, Callable::Txn, t.params.len(), normal);
                e.row.signature = signature;
                e.row.text = text;
                entries.push(e);
            }
            BundleItem::Fn(_) => {}
            BundleItem::Feed(_) | BundleItem::Aggregate(_) => {
                return Err(Error::invalid(
                    "only queries, transactions and functions register in this engine",
                ));
            }
        }
    }
    let mut seen = BTreeSet::new();
    if let Some(e) = entries.iter().find(|e| !seen.insert(e.name.as_str())) {
        return Err(Error::invalid(format!(
            "`{}` is declared twice in the registration",
            e.name
        )));
    }
    for (i, e) in entries.iter_mut().enumerate() {
        e.row.ordinal = i64::try_from(i + 1).expect("a registration holds fewer than 2^63 calls");
    }
    Ok(Checked {
        name: r.name.clone(),
        schema: extended.map_or_else(|| over.schema.clone(), |(s, _)| s),
        entries,
        header: r.header.clone(),
        current,
    })
}

/// Returns the text `span` covers in `text`.
fn spanned(text: &str, span: adtb_lang::span::Span) -> &str {
    text.get(span.start as usize..span.end as usize)
        .unwrap_or_default()
        .trim_end()
}

/// Returns the entry of a checked query or transaction `name` of `arity` declared parameters.
fn entry(name: &str, kind: Callable, arity: usize, n: NormalQuery) -> Entry {
    let (params, unit) = if arity == 0 {
        (Vec::new(), n.params.first().copied())
    } else {
        (n.params.iter().copied().map(Some).collect(), None)
    };
    let (writes, retry) = shape(&n.terms, n.body);
    let types = params
        .iter()
        .flatten()
        .map(|&v| n.terms.var_info(v).ty.clone())
        .collect();
    Entry {
        name: name.to_owned(),
        kind,
        row: CallRow {
            kind: kind.ctor(),
            name: name.to_owned(),
            ordinal: 0,
            signature: String::new(),
            text: String::new(),
        },
        terms: Arc::new(n.terms),
        body: n.body,
        params,
        types,
        unit,
        writes,
        retry,
    }
}

/// Returns whether `t` writes and the `retry n` of the `transact` block it is.
fn shape(terms: &Terms, t: Tm) -> (bool, Option<u64>) {
    let retry = match terms.kind(t) {
        Kind::Transact(_, o) => o.retry,
        _ => None,
    };
    (terms.writes(t), retry)
}

/// Returns the entry that calls the schema function `f`, imported into a registration that reads
/// the modules of `vis`.
fn imported(schema: &Schema, program: &Program, vis: &Visible, f: &QName) -> Result<Entry, Error> {
    let failed = |m: String| Error::invalid(format!("fn `{}`: {m}", f.name.text));
    let Ok(ValueDef::Function(id)) = lookup_value(schema, vis, f) else {
        return Err(failed("no schema function of this name to import".into()));
    };
    let def = &schema.functions[id.0 as usize];
    if !def.type_params.is_empty() {
        return Err(failed("a generic function is not called by ordinal".into()));
    }
    let names: Vec<String> = (0..def.params.len()).map(|i| format!("arg{i}")).collect();
    let text = format!(
        "{}::{}({})",
        schema.module(def.module).name,
        def.name,
        names.join(", ")
    );
    let items = parse_adhoc(&text, Options::default())
        .ok()
        .map_err(|ds| failed(joined(ds.iter().map(|d| d.message.as_str()))))?;
    let types: BTreeMap<String, Ty> = names
        .iter()
        .cloned()
        .zip(def.params.iter().map(|(_, t)| t.clone()))
        .collect();
    let adhoc = check_adhoc(schema, program, &items, &types)
        .map_err(|ds| failed(joined(ds.iter().map(|d| d.message.as_str()))))?;
    let normal = norm::adhoc(schema, program, &adhoc).map_err(no_normal)?;
    let [body] = normal.items.as_slice() else {
        return Err(no_normal("a call is one item"));
    };
    let (writes, retry) = shape(&normal.terms, *body);
    let shown: Vec<String> = def
        .params
        .iter()
        .map(|(n, t)| format!("{n}: {}", schema.show_ty(t)))
        .collect();
    let written = match &f.module {
        Some(m) => format!("{}::{}", m.text, f.name.text),
        None => f.name.text.clone(),
    };
    Ok(Entry {
        name: f.name.text.clone(),
        kind: Callable::Fn,
        row: CallRow {
            kind: Callable::Fn.ctor(),
            name: f.name.text.clone(),
            ordinal: 0,
            signature: format!("({}) -> {}", shown.join(", "), schema.show_ty(&def.result)),
            text: format!("import fn {written}"),
        },
        types: def.params.iter().map(|(_, t)| t.clone()).collect(),
        params: names
            .iter()
            .map(|n| normal.params.get(n).copied())
            .collect(),
        unit: None,
        body: *body,
        terms: Arc::new(normal.terms),
        writes,
        retry,
    })
}

fn no_normal(e: impl std::fmt::Debug) -> Error {
    Error::rejected(RejectReason::Internal {
        what: format!("a checked registered call has no normal form: {e:?}"),
    })
}

fn joined<'a>(ms: impl Iterator<Item = &'a str>) -> String {
    ms.collect::<Vec<_>>().join("\n")
}

/// A registration of an open database, by which its callables are called. It does not keep the
/// database open: once the database closes, every call through it is refused.
#[derive(Debug)]
pub struct Registered<R: Runtime = adtb_sys::os::OsRuntime> {
    engine: Weak<Engine<R>>,
    checked: Arc<Checked>,
}

impl<R: Runtime> Clone for Registered<R> {
    fn clone(&self) -> Self {
        Registered {
            engine: self.engine.clone(),
            checked: self.checked.clone(),
        }
    }
}

impl<R: Runtime> Database<R> {
    /// Returns the registration `name` the database was opened with, binding to it and renewing
    /// its lease when it is a bundle.
    ///
    /// Fails with `Retired`, `BundleSuperseded` or `BundleExpired` for a bundle that is, and with
    /// `UnknownBundle` for a name no registration of this open has.
    pub fn registered(&self, name: &str) -> Result<Registered<R>, Error> {
        let _open = self.engine.enter()?;
        self.engine.bind(name, "")?;
        self.engine
            .registered
            .iter()
            .find(|c| c.name == name)
            .map(|c| Registered {
                engine: Arc::downgrade(&self.engine),
                checked: c.clone(),
            })
            .ok_or_else(|| {
                Error::rejected(RejectReason::UnknownBundle {
                    bundle: name.to_owned(),
                })
            })
    }
}

impl<R: Runtime> Registered<R> {
    pub fn name(&self) -> &str {
        &self.checked().name
    }

    /// Returns the version a bundle's header pins; `None` for a registration that is no bundle.
    pub fn pins(&self) -> Option<u64> {
        self.checked().header.as_ref().map(|h| h.pins)
    }

    /// Returns the ordinal of the query, transaction or imported function `name`, from 1 in
    /// declaration order.
    pub fn ordinal(&self, name: &str) -> Option<u32> {
        self.checked()
            .entries
            .iter()
            .position(|e| e.name == name)
            .map(|i| u32::try_from(i + 1).expect("a registration holds fewer than 2^32 calls"))
    }

    /// Call the query, transaction or imported function `ordinal` with `args`, in parameter
    /// order, as one transaction of its own.
    ///
    /// A query or function only reads and runs as a read-only transaction. A transaction is
    /// rerun on a serialization failure up to `opts.retry`, else its `retry n`, else the retry
    /// limit.
    pub fn call(&self, ordinal: u32, args: &[Val], opts: &CallOptions) -> Result<Called, Error> {
        let e = self.engine()?;
        let _open = e.enter()?;
        let entry = self.entry(&e, ordinal)?;
        let caller = e.principal(opts.assert.as_ref())?;
        let p = prepared(self.checked(), entry, args, caller)?;
        let done = e.call(&p, opts)?;
        let value = one(&e, entry, done.value);
        Ok(match entry.kind {
            Callable::Txn => Called::Committed(Committed {
                value,
                position: done.position,
                attempts: done.attempts,
            }),
            Callable::Query | Callable::Fn => Called::Rows(value),
        })
    }

    /// Call the query or function `ordinal` with `args` as [`call`](Registered::call) does.
    /// Returns its result as record batches of one schema, as the wire's Arrow format maps the
    /// result type.
    pub fn call_arrow(
        &self,
        ordinal: u32,
        args: &[Val],
        opts: &CallOptions,
    ) -> Result<ArrowStream, Error> {
        let rows = match self.call(ordinal, args, opts)? {
            Called::Rows(rows) => rows,
            Called::Committed(done) => done.value,
        };
        arrow::stream(&self.checked.schema, rows)
    }

    fn checked(&self) -> &Checked {
        &self.checked
    }

    /// Returns the engine, refusing a call once the database is closed.
    fn engine(&self) -> Result<Arc<Engine<R>>, Error> {
        self.engine.upgrade().ok_or_else(Error::closed)
    }

    /// Returns the callable `ordinal`, refusing a call of a bundle that is retired, superseded
    /// or expired, renewing its lease in `e` when a renewal is due, and refusing a call of a
    /// bundle pinned before the version the database is at.
    fn entry(&self, e: &Engine<R>, ordinal: u32) -> Result<&Entry, Error> {
        let checked = self.checked();
        let entry = usize::try_from(ordinal)
            .ok()
            .and_then(|i| checked.entries.get(i.checked_sub(1)?))
            .ok_or_else(|| Error::rejected(RejectReason::UnknownQuery))?;
        e.bind(&checked.name, &entry.name)?;
        if !checked.current {
            return Err(Error::rejected(RejectReason::Untranslatable {
                query: format!("{}::{}", checked.name, entry.name),
                detail: "the bundle pins an earlier version than the database is at, and this \
                         engine serves a bundle only at the version the database is at"
                    .into(),
            }));
        }
        Ok(entry)
    }
}

impl<R: Runtime> AdHocTransaction<'_, R> {
    /// Call the query, transaction or imported function `ordinal` of `reg` with `args` as one
    /// call of the transaction, for the transaction's principal. A registered transaction's
    /// block commits with the transaction.
    ///
    /// A call that fails applies nothing and leaves the transaction open, except on a
    /// serialization failure, which aborts the transaction.
    pub fn call(
        &mut self,
        reg: &Registered<R>,
        ordinal: u32,
        args: &[Val],
        opts: &TxnCallOptions,
    ) -> Result<InTxn, Error> {
        if !std::ptr::eq(self.engine, reg.engine.as_ptr()) {
            return Err(Error::invalid(
                "a registration calls only in a transaction of its own database",
            ));
        }
        let entry = reg.entry(self.engine, ordinal)?;
        let p = prepared(reg.checked(), entry, args, self.caller.clone())?;
        if self.read_only && p.writes {
            return Err(Error::invalid("a read-only transaction does not write"));
        }
        let value = one(self.engine, entry, self.run(&p, opts)?);
        Ok(match entry.kind {
            Callable::Txn => InTxn::Pending(value),
            Callable::Query | Callable::Fn => InTxn::Rows(value),
        })
    }
}

/// Returns the call of `entry` with `args` for `caller`.
fn prepared(
    checked: &Checked,
    entry: &Entry,
    args: &[Val],
    caller: Val,
) -> Result<Prepared, Error> {
    if args.len() != entry.params.len() {
        return Err(Error::invalid(format!(
            "`{}` takes {} arguments, not {}",
            entry.name,
            entry.params.len(),
            args.len()
        )));
    }
    let mut params: Vec<(Var, Val)> = entry
        .params
        .iter()
        .zip(args)
        .filter_map(|(x, v)| x.map(|x| (x, v.clone())))
        .collect();
    params.extend(entry.unit.map(|u| (u, Val::record(Vec::new()))));
    Ok(Prepared {
        schema: checked.schema.clone(),
        terms: entry.terms.clone(),
        items: vec![entry.body],
        commands: Vec::new(),
        params,
        caller,
        writes: entry.writes,
        retry: entry.retry,
        deferrable: false,
    })
}

/// Returns the one result of a call of `entry` among `rows`, the rows of its one item.
fn one<R: Runtime>(e: &Engine<R>, entry: &Entry, mut out: Vec<DynRows>) -> DynRows {
    out.pop().unwrap_or_else(|| {
        rows(
            &e.schema,
            entry.terms.ty(entry.body).clone(),
            vec![Val::record(Vec::new())],
        )
    })
}
