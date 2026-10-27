//! Registered queries and transactions: checked and normalised when the database opens, called
//! by ordinal with arguments, each for the principal of its call.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::ast::{BundleItem, FnDecl, Import, Operator, OperatorKind, QName};
use adtb_lang::check::{Program, check_adhoc, check_query_in, check_txn_in};
use adtb_lang::evolve::Source;
use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::norm::{self, NormalQuery};
use adtb_lang::parse::{Options, parse_adhoc, parse_bundle_body};
use adtb_lang::resolve::{Visible, lookup_value};
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::Ty;

use crate::exec::Val;
use crate::runtime::Runtime;
use crate::write::Bounds;

use super::error::{Error, RejectReason};
use super::text::{DynRows, Prepared, rows};
use super::txn::{AdHocTransaction, Committed, TxnCallOptions};
use super::{CallOptions, Database, Engine, OpenError};

/// The queries, transactions and imported schema functions one unit registers: the imports and
/// items of a bundle file, without its header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    name: String,
    text: String,
}

impl Registration {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Registration {
        Registration {
            name: name.into(),
            text: text.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
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
    /// The variable of the unit parameter a body with no declared parameter takes.
    unit: Option<Var>,
    writes: bool,
    retry: Option<u64>,
}

/// A registration whose every callable checked.
#[derive(Debug)]
pub(super) struct Checked {
    name: String,
    /// The schema its callables run over: the database's, with the registration's functions.
    schema: Arc<Schema>,
    entries: Vec<Entry>,
}

/// The schema a registration checks against and the program of its functions.
struct Over<'a> {
    source: &'a Source,
    schema: &'a Arc<Schema>,
    program: &'a Program,
}

/// Check and normalise every registration of `regs` against `program` of `schema`, the schema
/// `source` builds, all or none.
pub(super) fn register(
    source: &Source,
    schema: &Arc<Schema>,
    program: &Program,
    regs: &[Registration],
) -> Result<Vec<Checked>, OpenError> {
    let over = Over {
        source,
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
        out.push(checked(&over, r).map_err(refused)?);
    }
    Ok(out)
}

fn checked(over: &Over<'_>, r: &Registration) -> Result<Checked, Error> {
    let (imports, items) = parse_bundle_body(&r.text, Options::default())
        .ok()
        .map_err(|ds| Error::invalid(joined(ds.iter().map(|d| d.message.as_str()))))?;
    let mut vis = Visible {
        modules: Vec::new(),
    };
    for i in &imports {
        if let Import::Module(m) = i {
            let id = over
                .schema
                .module_named(&m.text)
                .filter(|&id| !over.schema.module(id).engine)
                .ok_or_else(|| Error::invalid(format!("no module `{}` to import", m.text)))?;
            vis.modules.push(id);
        }
    }
    let fns: Vec<&FnDecl> = items
        .iter()
        .filter_map(|i| match i {
            BundleItem::Fn(f) => Some(f),
            _ => None,
        })
        .collect();
    let extended = with_functions(over, &vis, &fns, &r.text)?;
    let (schema, program) = match &extended {
        Some((s, p)) => (s, p),
        None => (over.schema, over.program),
    };
    let mut entries = Vec::new();
    for i in &imports {
        if let Import::Fn(f) = i {
            entries.push(imported(schema, program, &vis, f)?);
        }
    }
    for item in &items {
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
                entries.push(entry(&q.name.text, Callable::Query, q.params.len(), normal));
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
                entries.push(entry(&t.name.text, Callable::Txn, t.params.len(), normal));
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
    Ok(Checked {
        name: r.name.clone(),
        schema: extended.map_or_else(|| over.schema.clone(), |(s, _)| s),
        entries,
    })
}

/// Returns the schema and program of `over` with the bundle functions `fns` of `text` added to
/// the first module of `vis`, `None` when there are none. Refuses a function whose name is a
/// schema function's that `vis` sees, and one that fails to check.
fn with_functions(
    over: &Over<'_>,
    vis: &Visible,
    fns: &[&FnDecl],
    text: &str,
) -> Result<Option<(Arc<Schema>, Program)>, Error> {
    if fns.is_empty() {
        return Ok(None);
    }
    let Some(&home) = vis.modules.first() else {
        return Err(Error::invalid(
            "a registration with functions imports a module",
        ));
    };
    let module = &over.schema.module(home).name;
    let mut source = over.source.clone();
    for f in fns {
        let failed = |m: String| Error::invalid(format!("fn `{}`: {m}", f.name.text));
        let name = QName {
            module: None,
            name: f.name.clone(),
        };
        if let Ok(ValueDef::Function(_)) = lookup_value(over.schema, vis, &name) {
            return Err(failed(
                "a bundle function has the name of a schema function".into(),
            ));
        }
        let op = Operator {
            kind: OperatorKind::DefineFn((*f).clone()),
            span: f.span,
        };
        source
            .apply(module, &op, text)
            .map_err(|ds| failed(joined(ds.iter().map(|d| d.message.as_str()))))?;
    }
    let (schema, program) = source
        .check()
        .map_err(|ds| Error::invalid(joined(ds.iter().map(|d| d.diag.message.as_str()))))?;
    Ok(Some((Arc::new(schema), program)))
}

/// Returns the entry of a checked query or transaction `name` of `arity` declared parameters.
fn entry(name: &str, kind: Callable, arity: usize, n: NormalQuery) -> Entry {
    let (params, unit) = if arity == 0 {
        (Vec::new(), n.params.first().copied())
    } else {
        (n.params.iter().copied().map(Some).collect(), None)
    };
    let (writes, retry) = shape(&n.terms, n.body);
    Entry {
        name: name.to_owned(),
        kind,
        terms: Arc::new(n.terms),
        body: n.body,
        params,
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
    Ok(Entry {
        name: f.name.text.clone(),
        kind: Callable::Fn,
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

/// A registration of an open database, by which its callables are called.
#[derive(Debug)]
pub struct Registered<R: Runtime = adtb_sys::os::OsRuntime> {
    engine: Arc<Engine<R>>,
    at: usize,
}

impl<R: Runtime> Clone for Registered<R> {
    fn clone(&self) -> Self {
        Registered {
            engine: self.engine.clone(),
            at: self.at,
        }
    }
}

impl<R: Runtime> Database<R> {
    /// Returns the registration `name` the database was opened with, or `UnknownBundle`.
    pub fn registered(&self, name: &str) -> Result<Registered<R>, Error> {
        self.engine
            .registered
            .iter()
            .position(|c| c.name == name)
            .map(|at| Registered {
                engine: self.engine.clone(),
                at,
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

    /// Returns the ordinal of the query, transaction or imported function `name`.
    pub fn ordinal(&self, name: &str) -> Option<u32> {
        self.checked()
            .entries
            .iter()
            .position(|e| e.name == name)
            .map(|i| u32::try_from(i).expect("a registration holds fewer than 2^32 calls"))
    }

    /// Call the query, transaction or imported function `ordinal` with `args`, in parameter
    /// order, as one transaction of its own.
    ///
    /// A query or function only reads and runs as a read-only transaction. A transaction is
    /// rerun on a serialization failure up to `opts.retry`, else its `retry n`, else the retry
    /// limit.
    pub fn call(&self, ordinal: u32, args: &[Val], opts: &CallOptions) -> Result<Called, Error> {
        let e = &*self.engine;
        let entry = self.entry(ordinal)?;
        let caller = e.principal(opts.assert.as_ref())?;
        let p = prepared(self.checked(), entry, args, caller)?;
        let done = e.call(&p, opts)?;
        let value = one(e, entry, done.value);
        Ok(match entry.kind {
            Callable::Txn => Called::Committed(Committed {
                value,
                position: done.position,
                attempts: done.attempts,
            }),
            Callable::Query | Callable::Fn => Called::Rows(value),
        })
    }

    fn checked(&self) -> &Checked {
        &self.engine.registered[self.at]
    }

    fn entry(&self, ordinal: u32) -> Result<&Entry, Error> {
        usize::try_from(ordinal)
            .ok()
            .and_then(|i| self.checked().entries.get(i))
            .ok_or_else(|| Error::rejected(RejectReason::UnknownQuery))
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
        if !std::ptr::eq(self.engine, &*reg.engine) {
            return Err(Error::invalid(
                "a registration calls only in a transaction of its own database",
            ));
        }
        let entry = reg.entry(ordinal)?;
        let p = prepared(reg.checked(), entry, args, self.caller.clone())?;
        if self.read_only && p.writes {
            return Err(Error::invalid("a read-only transaction does not write"));
        }
        let bounds = Bounds {
            fix_rounds: opts.fix_rounds,
        };
        let value = one(self.engine, entry, self.run(&p, opts, bounds)?);
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
