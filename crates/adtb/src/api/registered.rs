//! Registered queries and transactions: checked and normalised when the database opens, called
//! by ordinal with arguments, each for the principal of its call.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};

use adtb_lang::ast::{BundleItem, FnDecl, Import, QName};
use adtb_lang::check::{Bundles, Program, check_adhoc};
use adtb_lang::evolve::Source;
use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::norm::{self, NormalQuery};
use adtb_lang::parse::{Options, parse_adhoc, parse_bundle, parse_bundle_body, parse_migration};
use adtb_lang::schema::Schema;
use adtb_lang::signature::{self, Item};
use adtb_lang::translate::{Map, Pinned, Untranslatable};
use adtb_lang::types::{FnId, Ty};

use crate::catalog::{self, Catalog};
use crate::exec::{Priority, Val};
use crate::runtime::Runtime;

use super::arrow::{self, ArrowStream};
use super::bundle::{self, CallRow, EDITION, Header, Stored};
use super::error::{Error, RejectReason};
use super::pinned;
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
    /// Fail with `Invalid` when `text` does not parse as a bundle file, and when its header
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
    /// Its translation to the version the database is at, for a registration checked at an
    /// earlier one.
    served: Option<Served>,
}

/// A callable translated to the version the database is at, its variables as [`Entry`]'s.
#[derive(Debug)]
struct Served {
    /// The schema it runs over.
    schema: Arc<Schema>,
    terms: Arc<Terms>,
    body: Tm,
    params: Vec<Option<Var>>,
    unit: Option<Var>,
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
}

impl Checked {
    /// Returns the header and calls of a bundle's registration.
    pub(super) fn bundle(&self) -> Option<(&Header, Vec<CallRow>)> {
        let calls = self.entries.iter().map(|e| e.row.clone()).collect();
        self.header.as_ref().map(|h| (h, calls))
    }

    /// Returns the catalog reader of a bundle's registration, at the version it pins.
    fn reader(&self) -> Option<catalog::Reader> {
        let h = self.header.as_ref()?;
        let mut reads = Reads::default();
        for e in &self.entries {
            reads.add(&self.schema, &e.terms, e.body);
        }
        Some(reads.reader(&self.schema, &h.name, h.pins, h.hides_new))
    }

    /// Returns `rows`, the result of a call of `entry`, encoded at the version the registration
    /// is checked at.
    fn encoded(&self, entry: &Entry, rows: DynRows) -> Result<DynRows, Error> {
        let Some(served) = &entry.served else {
            return Ok(rows);
        };
        let map = Map::new(&self.schema, &served.schema);
        let ty = match entry.terms.ty(entry.body) {
            Ty::Set(t) | Ty::NonEmpty(t) => t.as_ref().clone(),
            t => t.clone(),
        };
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for v in rows {
            let v = pinned::back(&map, &ty, &v?);
            if seen.insert(v.clone()) {
                out.push(v);
            }
        }
        Ok(super::text::rows(&self.schema, ty, out))
    }

    /// Record the queries and transactions of a bundle whose calls run in `bundles`.
    pub(super) fn explained(&self, bundles: &mut Bundles) {
        let translated = self.entries.iter().any(|e| e.served.is_some());
        let Some(h) = self.header.as_ref().filter(|_| !translated) else {
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
        let version = catalog.source().version;
        let Some(h) = &r.header else {
            let c = checked(&over, r).map_err(refused)?;
            pinned_within(&c, version).map_err(refused)?;
            out.push(c);
            continue;
        };
        let at_pin = pinned(catalog, h).map_err(refused)?;
        let mut c = match &at_pin {
            Some(pin) => checked(&pin.over(), r),
            None => checked(&over, r),
        }
        .map_err(refused)?;
        let (_, calls) = c.bundle().expect("a bundle's registration has a header");
        bundle::admitted(stored, h, &calls).map_err(refused)?;
        if let Some(pin) = &at_pin {
            let served = translations(&c, &pin.source, schema, catalog.source(), program).map_err(
                |(query, u)| refused(untranslatable(stored, h, now, s, &c.name, &query, &u)),
            )?;
            for (e, s) in c.entries.iter_mut().zip(served) {
                e.served = Some(s);
            }
        }
        pinned_within(&c, version).map_err(refused)?;
        if let Some(ctor) = stale(catalog, &c).map_err(refused)? {
            return Err(refused(bundle::stale(stored, h, now, s, ctor)));
        }
        out.push(c);
    }
    Ok(out)
}

/// Returns the refusal of registering `h`, whose callable `query` of bundle `name` has no
/// translation for `u`, at `now` under the file settings `s`: `BundleExpired` for a bundle whose
/// lease ran out or that is superseded, else `Untranslatable`.
fn untranslatable(
    stored: &Stored,
    h: &Header,
    now: i64,
    s: &Settings,
    name: &str,
    query: &str,
    u: &Untranslatable,
) -> Error {
    let query = format!("{name}::{query}");
    if bundle::lapsed(stored, h, now, s) {
        return Error::rejected(RejectReason::BundleExpired {
            bundle: h.name.clone(),
            query,
        });
    }
    Error::rejected(RejectReason::Untranslatable {
        query,
        detail: u.to_string(),
    })
}

/// Refuse a callable of `c` with an `at` block pinned at a version after `version`.
fn pinned_within(c: &Checked, version: u64) -> Result<(), Error> {
    for e in &c.entries {
        let mut seen = BTreeSet::new();
        let mut todo = vec![e.body];
        while let Some(t) = todo.pop() {
            if !seen.insert(t) {
                continue;
            }
            if let Kind::At { pins: Some(v), .. } = e.terms.kind(t)
                && *v > version
            {
                return Err(Error::invalid(format!(
                    "`{}` reads the past at version {v}, and the database is at version {version}",
                    e.name
                )));
            }
            todo.extend(e.terms.children(t));
        }
    }
    Ok(())
}

/// Refuse an open whose bundles pin a version before the file's and would read a constructor
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
        let Ok(c) = checked(&pin.over(), r) else {
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
    let Some(reader) = c.reader() else {
        return Ok(None);
    };
    catalog.stale(&reader).map_err(catalog_error)
}

fn catalog_error(e: catalog::Error) -> Error {
    match e {
        catalog::Error::Refused(why) | catalog::Error::Unaccepted(_, why) => {
            Error::invalid(why.join("\n"))
        }
        other => Error::rejected(RejectReason::Internal {
            what: format!("{other:?}"),
        }),
    }
}

/// A live bundle, checked at the version it pins, with the source of that version.
pub(super) struct Live {
    checked: Checked,
    source: Source,
}

/// Register as a reader of `catalog` each bundle of `regs` that `live` names, at the version it
/// pins. Returns those bundles.
pub(super) fn readers(
    catalog: &mut Catalog,
    regs: &[Registration],
    live: &BTreeSet<String>,
) -> Result<Vec<Live>, OpenError> {
    let mut out = Vec::new();
    let schema = Arc::new(catalog.schema().clone());
    let program = catalog.program().clone();
    for r in regs {
        let Some(h) = r.header.as_ref().filter(|h| live.contains(&h.name)) else {
            continue;
        };
        let refused = |reason: Error| OpenError::RegistrationRefused {
            bundle: r.name.clone(),
            reason,
        };
        let (c, source) = match pinned(catalog, h).map_err(refused)? {
            Some(pin) => (checked(&pin.over(), r), pin.source),
            None => (
                checked(
                    &Over {
                        source: catalog.source(),
                        schema: &schema,
                        program: &program,
                    },
                    r,
                ),
                catalog.source().clone(),
            ),
        };
        let c = c.map_err(refused)?;
        let reader = c.reader().expect("a bundle's registration has a header");
        catalog
            .register(reader)
            .map_err(|e| refused(catalog_error(e)))?;
        out.push(Live { checked: c, source });
    }
    Ok(out)
}

/// Refuse migration `text`, applied to the version `catalog` is at, when a query or transaction
/// of a bundle of `live` has no translation to the version it gives. A migration that does not
/// apply or check is left for the catalog to refuse.
pub(super) fn recheck(catalog: &Catalog, text: &str, live: &[Live]) -> Result<(), RejectReason> {
    if live.is_empty() {
        return Ok(());
    }
    let Ok(file) = parse_migration(text, Options::default()).ok() else {
        return Ok(());
    };
    let Ok(next) = catalog.source().migrate(&file, text) else {
        return Ok(());
    };
    let Ok((schema, program)) = next.check() else {
        return Ok(());
    };
    let schema = Arc::new(schema);
    for l in live {
        if let Err((query, u)) = translations(&l.checked, &l.source, &schema, &next, &program) {
            let pins = l.checked.header.as_ref().map_or(0, |h| h.pins);
            return Err(RejectReason::SchemaRefused {
                operator: u.operator.clone().unwrap_or_default(),
                detail: format!(
                    "bundle {} pins {pins}: `{query}` has no translation: {}",
                    l.checked.name, u.reason
                ),
            });
        }
    }
    Ok(())
}

/// Returns the translation of each callable of `c`, checked at the version of `pin`, to `now`,
/// the schema of `now_source` checked as `program`. Fail with the name of the first callable
/// that has none, and why.
fn translations(
    c: &Checked,
    pin: &Source,
    now: &Arc<Schema>,
    now_source: &Source,
    program: &Program,
) -> Result<Vec<Served>, (String, Untranslatable)> {
    let Some(first) = c.entries.first() else {
        return Ok(Vec::new());
    };
    let hides = c.header.as_ref().is_some_and(|h| h.hides_new);
    let p = Pinned::new(&c.schema, pin, now, now_source, program, hides)
        .map_err(|u| (first.name.clone(), u))?;
    let mut out = Vec::with_capacity(c.entries.len());
    for e in &c.entries {
        let q = NormalQuery {
            terms: (*e.terms).clone(),
            params: e.params.iter().flatten().copied().chain(e.unit).collect(),
            body: e.body,
        };
        let t = p.query(&q).map_err(|u| (e.name.clone(), u))?;
        let mut vars = t.params.into_iter();
        let mut next = || vars.next().expect("a translation keeps every parameter");
        let params = e.params.iter().map(|x| x.map(|_| next())).collect();
        let unit = e.unit.map(|_| next());
        out.push(Served {
            schema: now.clone(),
            terms: Arc::new(t.terms),
            body: t.body,
            params,
            unit,
        });
    }
    Ok(out)
}

/// Check and normalise registration `r` over `over`.
fn checked(over: &Over<'_>, r: &Registration) -> Result<Checked, Error> {
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
    let decl = |span, name: &adtb_lang::ast::Ident, result: &adtb_lang::ast::Type| {
        let signature = r
            .text
            .get(name.span.end as usize..result.span.end as usize)
            .unwrap_or_default()
            .trim()
            .to_owned();
        (signature, format!("{}{functions}", spanned(&r.text, span)))
    };
    let calls =
        signature::calls(schema, program, &vis, &imports, &items).map_err(Error::diagnostic)?;
    let mut entries = Vec::new();
    for c in calls {
        let mut e = match c.item {
            Item::Fn(f, id) => imported(schema, program, f, id)?,
            Item::Query(q, checked) => {
                let normal = norm::query(schema, program, &checked).map_err(no_normal)?;
                let (signature, text) = decl(q.span, &q.name, &q.result);
                let mut e = entry(&q.name.text, Callable::Query, q.params.len(), normal);
                e.row.signature = signature;
                e.row.text = text;
                e
            }
            Item::Txn(t, checked) => {
                let normal = norm::txn(schema, program, &checked).map_err(no_normal)?;
                let (signature, text) = decl(t.span, &t.name, &t.result);
                let mut e = entry(&t.name.text, Callable::Txn, t.params.len(), normal);
                e.row.signature = signature;
                e.row.text = text;
                e
            }
        };
        e.row.ordinal = i64::from(c.signature.ordinal);
        entries.push(e);
    }
    Ok(Checked {
        name: r.name.clone(),
        schema: extended.map_or_else(|| over.schema.clone(), |(s, _)| s),
        entries,
        header: r.header.clone(),
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
        served: None,
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

/// Returns the entry that calls the schema function `id`, imported by the name `f`.
fn imported(schema: &Schema, program: &Program, f: &QName, id: FnId) -> Result<Entry, Error> {
    let failed = |m: String| Error::invalid(format!("fn `{}`: {m}", f.name.text));
    let def = schema.function(id);
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
    let adhoc = check_adhoc(schema, program, &items, &types).map_err(|mut ds| {
        if let Some(first) = ds.first_mut() {
            first.message = format!("fn `{}`: {}", f.name.text, first.message);
        }
        Error::diagnostics(ds)
    })?;
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
        served: None,
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
    /// Fail with `Retired`, `BundleSuperseded` or `BundleExpired` for a bundle that is, and with
    /// `UnknownBundle` for a name no registration of this open has.
    pub fn registered(&self, name: &str) -> Result<Registered<R>, Error> {
        let _open = self.engine.enter()?;
        self.engine.bind(name)?;
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
        let (entry, p, due) = self.admitted(&e, ordinal, args, false, || {
            e.principal(opts.assert.as_ref())
        })?;
        let done = e.call(&p, opts, || {
            if due {
                e.renew(&self.checked.name);
            }
        })?;
        let value = self.checked.encoded(entry, one(&e, entry, done.value))?;
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

    fn admitted(
        &self,
        e: &Engine<R>,
        ordinal: u32,
        args: &[Val],
        read_only: bool,
        caller: impl FnOnce() -> Result<Val, Error>,
    ) -> Result<(&Entry, Prepared, bool), Error> {
        let checked = self.checked();
        let entry = usize::try_from(ordinal)
            .ok()
            .and_then(|i| checked.entries.get(i.checked_sub(1)?))
            .ok_or_else(|| Error::rejected(RejectReason::UnknownQuery))?;
        let due = e.admit(&checked.name, &entry.name)?;
        let caller = caller()?;
        arguments(checked, entry, args)?;
        if read_only && entry.writes {
            return Err(Error::invalid("a read-only transaction does not write"));
        }
        let p = prepared(checked, entry, args, caller);
        Ok((entry, p, due))
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
        let (entry, p, due) = reg.admitted(self.engine, ordinal, args, self.read_only, || {
            Ok(self.caller.clone())
        })?;
        let e = self.engine;
        let rows = self.run(&p, opts, || {
            if due {
                e.renew(&reg.checked.name);
            }
        })?;
        let value = reg.checked.encoded(entry, one(e, entry, rows))?;
        Ok(match entry.kind {
            Callable::Txn => InTxn::Pending(value),
            Callable::Query | Callable::Fn => InTxn::Rows(value),
        })
    }
}

fn arguments(checked: &Checked, entry: &Entry, args: &[Val]) -> Result<(), Error> {
    if args.len() != entry.params.len() {
        return Err(Error::invalid(format!(
            "`{}` takes {} arguments, not {}",
            entry.name,
            entry.params.len(),
            args.len()
        )));
    }
    for (i, (ty, value)) in entry.types.iter().zip(args).enumerate() {
        if !super::value::valid(&checked.schema, ty, value) {
            return Err(Error::invalid(format!(
                "argument {} of `{}` is not a value of its declared parameter type",
                i + 1,
                entry.name
            )));
        }
    }
    Ok(())
}

/// Returns the call of `entry` with `args` for `caller`, through its translation when it has
/// one, its arguments carried to the version the database is at.
fn prepared(checked: &Checked, entry: &Entry, args: &[Val], caller: Val) -> Prepared {
    let (schema, terms, body, vars, unit) = match &entry.served {
        Some(s) => (&s.schema, &s.terms, s.body, &s.params, s.unit),
        None => (
            &checked.schema,
            &entry.terms,
            entry.body,
            &entry.params,
            entry.unit,
        ),
    };
    let map = entry
        .served
        .as_ref()
        .map(|s| Map::new(&checked.schema, &s.schema));
    let carried = |ty: &Ty, v: &Val| match &map {
        Some(m) => pinned::forward(m, ty, v),
        None => v.clone(),
    };
    let mut params: Vec<(Var, Val)> = vars
        .iter()
        .zip(entry.types.iter().zip(args))
        .filter_map(|(x, (ty, v))| x.map(|x| (x, carried(ty, v))))
        .collect();
    params.extend(unit.map(|u| (u, Val::record(Vec::new()))));
    Prepared {
        schema: schema.clone(),
        terms: terms.clone(),
        items: vec![body],
        commands: Vec::new(),
        params,
        caller,
        writes: entry.writes,
        retry: entry.retry,
        deferrable: false,
    }
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

/// A call of a registration checked against its signature, for a task to run.
pub(super) struct Bound {
    pub(super) prepared: Prepared,
    pub(super) priority: Priority,
    /// Whether the call renews the bundle's lease once it starts.
    pub(super) due: bool,
    checked: Arc<Checked>,
    entry: usize,
}

impl<R: Runtime> Registered<R> {
    /// Returns the call of `ordinal` with `args` for `caller`, checked as
    /// [`call`](Registered::call) checks it.
    pub(super) fn bound(
        &self,
        e: &Engine<R>,
        ordinal: u32,
        args: &[Val],
        caller: Val,
    ) -> Result<Bound, Error> {
        let (_, prepared, due) = self.admitted(e, ordinal, args, false, || Ok(caller))?;
        Ok(Bound {
            prepared,
            priority: self
                .checked
                .header
                .as_ref()
                .map_or(Priority::Regular, |h| h.priority),
            due,
            entry: ordinal as usize - 1,
            checked: self.checked.clone(),
        })
    }

    pub(super) fn engine_ref(&self) -> &Weak<Engine<R>> {
        &self.engine
    }
}

impl Bound {
    pub(super) fn name(&self) -> &str {
        &self.checked.name
    }

    /// Returns the schema the registration is checked against.
    pub(super) fn schema(&self) -> &Arc<Schema> {
        &self.checked.schema
    }

    /// Returns the one result among `rows`, the rows of the call's one item, encoded at the
    /// version the registration is checked at.
    pub(super) fn result<R: Runtime>(
        &self,
        e: &Engine<R>,
        rows: Vec<DynRows>,
    ) -> Result<DynRows, Error> {
        let entry = &self.checked.entries[self.entry];
        self.checked.encoded(entry, one(e, entry, rows))
    }

    /// Returns `elems`, elements of type `ty` of the call's set result over `schema`, encoded at
    /// the version the registration is checked at.
    pub(super) fn elements(
        &self,
        schema: &Schema,
        ty: Ty,
        elems: Vec<Val>,
    ) -> Result<DynRows, Error> {
        let entry = &self.checked.entries[self.entry];
        self.checked.encoded(entry, rows(schema, ty, elems))
    }
}
