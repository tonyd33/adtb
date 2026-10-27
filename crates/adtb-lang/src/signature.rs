//! The callables of a bundle file with their ordinals and types, over the schema a replayed log
//! builds: what a code generator emits typed calls from.

use crate::ast::{self, BundleItem, FnDecl, Import, Operator, OperatorKind, QName};
use crate::check::{Program, check_query_in, check_txn_in};
use crate::evolve::Source;
use crate::parse::{Options, parse_bundle, parse_migration};
use crate::resolve::{Visible, lookup_value};
use crate::schema::{Schema, ValueDef};
use crate::types::Ty;

/// A migration of a log that does not apply, numbered from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayError {
    pub migration: u64,
    pub message: String,
}

/// Returns the source every database that applies `log`, in order, holds.
pub fn replay<'a>(log: impl IntoIterator<Item = &'a str>) -> Result<Source, ReplayError> {
    let mut source = Source::default();
    for (i, text) in log.into_iter().enumerate() {
        let failed = |message: String| ReplayError {
            migration: i as u64 + 1,
            message,
        };
        let file = parse_migration(text, Options::default())
            .ok()
            .map_err(|ds| failed(joined(ds.iter().map(|d| d.message.as_str()))))?;
        source = source.migrate(&file, text).map_err(|r| {
            let ops = r.operators.iter().map(|d| d.message.as_str());
            let recheck = r.recheck.iter().map(|d| d.diag.message.as_str());
            failed(joined(ops.chain(recheck)))
        })?;
    }
    Ok(source)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Query,
    Txn,
    /// A schema function the bundle imports.
    Fn,
}

/// One callable of a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// From 1: each `import fn` in import order, then each query and transaction in item order.
    pub ordinal: u32,
    pub name: String,
    pub kind: CallKind,
    pub params: Vec<(String, Ty)>,
    pub result: Ty,
}

/// A bundle file checked over the source of the version it pins.
#[derive(Debug)]
pub struct Bundle {
    pub header: ast::BundleHeader,
    /// The pinned version's schema, with the bundle's functions in its first imported module.
    pub schema: Schema,
    pub calls: Vec<Signature>,
}

/// Returns the bundle file `text` checked over `source`, which the caller replays to the version
/// the header pins.
///
/// Fails with the message of the first error: a file that does not parse, a module import of no
/// user module, a function that does not check, a callable that does not check or is declared
/// twice, a generic imported function, and a feed or aggregate, which take no ordinal here.
pub fn bundle(source: &Source, text: &str) -> Result<Bundle, String> {
    let file = parse_bundle(text, Options::default())
        .ok()
        .map_err(|ds| joined(ds.iter().map(|d| d.message.as_str())))?;
    let (base, base_program) = source.check().map_err(file_diags)?;
    let vis = visible(&base, &file.imports)?;
    let fns: Vec<&FnDecl> = file
        .items
        .iter()
        .filter_map(|i| match i {
            BundleItem::Fn(f) => Some(f),
            _ => None,
        })
        .collect();
    let (schema, program) =
        with_functions(source, &base, &vis, &fns, text)?.unwrap_or((base, base_program));
    let mut calls = Vec::new();
    for i in &file.imports {
        if let Import::Fn(f) = i {
            calls.push(imported(&schema, &vis, f)?);
        }
    }
    for item in &file.items {
        match item {
            BundleItem::Query(q) => {
                let checked = check_query_in(&schema, &program, vis.clone(), q)
                    .map_err(|d| format!("query `{}`: {}", q.name.text, d.message))?;
                let names = q.params.iter().map(|p| p.name.text.clone()).collect();
                let ty = checked.terms.ty(checked.term);
                calls.push(declared(&q.name.text, CallKind::Query, names, ty));
            }
            BundleItem::Txn(t) => {
                let checked = check_txn_in(&schema, &program, vis.clone(), t)
                    .map_err(|d| format!("txn `{}`: {}", t.name.text, d.message))?;
                let names = t.params.iter().map(|p| p.name.text.clone()).collect();
                let ty = checked.terms.ty(checked.term);
                calls.push(declared(&t.name.text, CallKind::Txn, names, ty));
            }
            BundleItem::Fn(_) => {}
            BundleItem::Feed(_) | BundleItem::Aggregate(_) => {
                return Err(
                    "only queries, transactions and functions register in this engine".into(),
                );
            }
        }
    }
    for (i, c) in calls.iter().enumerate() {
        if calls[..i].iter().any(|d| d.name == c.name) {
            return Err(format!("`{}` is declared twice in the bundle", c.name));
        }
    }
    for (i, c) in calls.iter_mut().enumerate() {
        c.ordinal = u32::try_from(i + 1).expect("a bundle holds fewer than 2^32 callables");
    }
    Ok(Bundle {
        header: file.header,
        schema,
        calls,
    })
}

/// Returns the user modules `imports` names, in import order.
pub fn visible(schema: &Schema, imports: &[Import]) -> Result<Visible, String> {
    let mut vis = Visible {
        modules: Vec::new(),
    };
    for i in imports {
        if let Import::Module(m) = i {
            let id = schema
                .module_named(&m.text)
                .filter(|&id| !schema.module(id).engine)
                .ok_or_else(|| format!("no module `{}` to import", m.text))?;
            vis.modules.push(id);
        }
    }
    Ok(vis)
}

/// Returns the schema and program of `source`, whose schema is `schema`, with the bundle
/// functions `fns` of `text` added to the first module of `vis`; `None` when there are none.
///
/// Refuses a function whose name is that of a schema function `vis` sees, and one that fails to
/// check.
pub fn with_functions(
    source: &Source,
    schema: &Schema,
    vis: &Visible,
    fns: &[&FnDecl],
    text: &str,
) -> Result<Option<(Schema, Program)>, String> {
    if fns.is_empty() {
        return Ok(None);
    }
    let Some(&home) = vis.modules.first() else {
        return Err("a registration with functions imports a module".into());
    };
    let module = &schema.module(home).name;
    let mut source = source.clone();
    for f in fns {
        let failed = |m: String| format!("fn `{}`: {m}", f.name.text);
        let name = QName {
            module: None,
            name: f.name.clone(),
        };
        if let Ok(ValueDef::Function(_)) = lookup_value(schema, vis, &name) {
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
    source.check().map(Some).map_err(file_diags)
}

/// Returns the signature of the schema function `f` a bundle seeing `vis` imports.
fn imported(schema: &Schema, vis: &Visible, f: &QName) -> Result<Signature, String> {
    let failed = |m: &str| format!("fn `{}`: {m}", f.name.text);
    let Ok(ValueDef::Function(id)) = lookup_value(schema, vis, f) else {
        return Err(failed("no schema function of this name to import"));
    };
    let def = schema.function(id);
    if !def.type_params.is_empty() {
        return Err(failed("a generic function is not called by ordinal"));
    }
    Ok(Signature {
        ordinal: 0,
        name: f.name.text.clone(),
        kind: CallKind::Fn,
        params: def.params.clone(),
        result: def.result.clone(),
    })
}

/// Returns the signature of a declared query or transaction of type `ty`, a function of its
/// parameters `names`, or of unit when it has none.
fn declared(name: &str, kind: CallKind, names: Vec<String>, ty: &Ty) -> Signature {
    let mut params = Vec::new();
    let mut ty = ty;
    for n in names {
        let Ty::Fn(p, r) = ty else {
            panic!("a checked callable is a function of each of its parameters")
        };
        params.push((n, (**p).clone()));
        ty = r;
    }
    if params.is_empty()
        && let Ty::Fn(_, r) = ty
    {
        ty = r;
    }
    Signature {
        ordinal: 0,
        name: name.to_owned(),
        kind,
        params,
        result: ty.clone(),
    }
}

fn file_diags(ds: Vec<crate::span::FileDiag>) -> String {
    joined(ds.iter().map(|d| d.diag.message.as_str()))
}

fn joined<'a>(ms: impl Iterator<Item = &'a str>) -> String {
    ms.collect::<Vec<_>>().join("\n")
}
