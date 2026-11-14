use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_lang::ast::{BundleItem, FnDecl, QName};
use adtb_lang::check::{Program, check_adhoc};
use adtb_lang::ids::CatFunctionId;
use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::norm::{self, NormalQuery};
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::schema::Schema;
use adtb_lang::signature::{self, Item};
use adtb_lang::types::{FnId, Ty};

use super::super::bundle::CallRow;
use super::super::cause::Broken;
use super::super::error::Error;
use super::super::reads::Reads;
use super::{Callable, Checked, Entry, Over, Registration, joined};

/// Check and normalise registration `r` over `over`.
pub(super) fn checked(over: &Over<'_>, r: &Registration) -> Result<Checked, Error> {
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
        .map(|(s, p)| (Arc::new(s), Arc::new(p)));
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
    let mut source_reads = Reads::default();
    for c in calls {
        let mut e = match c.item {
            Item::Fn(f, id) => {
                if let Some(Some(body)) = program.functions.get(id.0 as usize) {
                    source_reads.add_source(schema, program, &program.terms, *body);
                }
                imported(schema, program, f, id)?
            }
            Item::Query(q, checked) => {
                source_reads.add_source(schema, program, &checked.terms, checked.term);
                let normal = norm::query(schema, program, &checked).map_err(not_normal)?;
                let (signature, text) = decl(q.span, &q.name, &q.result);
                let mut e = declared(&q.name.text, Callable::Query, &c.signature, normal)?;
                e.row.signature = signature;
                e.row.text = text;
                e
            }
            Item::Txn(t, checked) => {
                source_reads.add_source(schema, program, &checked.terms, checked.term);
                let normal = norm::txn(schema, program, &checked).map_err(not_normal)?;
                let (signature, text) = decl(t.span, &t.name, &t.result);
                let mut e = declared(&t.name.text, Callable::Txn, &c.signature, normal)?;
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
        schema: extended
            .as_ref()
            .map_or_else(|| over.schema.clone(), |(s, _)| s.clone()),
        program: extended.map_or_else(|| over.program.clone(), |(_, p)| p),
        text: r.text.clone(),
        entries,
        source_reads,
        header: r.header.clone(),
    })
}

/// Returns the text `span` covers in `text`.
fn spanned(text: &str, span: adtb_lang::span::Span) -> &str {
    text.get(span.start as usize..span.end as usize)
        .unwrap_or_default()
        .trim_end()
}

/// What a call of an entry declares and binds.
struct Binding {
    params: Vec<Option<Var>>,
    types: Vec<Ty>,
    names: Vec<String>,
    result: Ty,
    unit: Option<Var>,
}

/// Returns the entry `name` of body `body` in `terms`, its row for the caller to complete. Fail
/// when the body expands past what the engine plans.
fn entry(
    name: &str,
    kind: Callable,
    terms: Terms,
    body: Tm,
    binding: Binding,
) -> Result<Entry, Error> {
    if !norm::expands_within_bound(&terms, &[body]) {
        return Err(Error::invalid(
            "the call's body expands past what the engine plans",
        ));
    }
    let (writes, retry) = shape(&terms, body);
    Ok(Entry {
        name: name.to_owned(),
        kind,
        row: CallRow {
            kind: kind.ctor(),
            name: name.to_owned(),
            ordinal: 0,
            signature: String::new(),
            text: String::new(),
        },
        terms: Arc::new(terms),
        body,
        params: binding.params,
        types: binding.types,
        names: binding.names,
        result: binding.result,
        unit: binding.unit,
        function: None,
        function_name: None,
        writes,
        retry,
        served: None,
    })
}

/// Returns the entry of a checked query or transaction `name` of signature `declared`.
fn declared(
    name: &str,
    kind: Callable,
    declared: &signature::Signature,
    n: NormalQuery,
) -> Result<Entry, Error> {
    let (params, unit) = if declared.params.is_empty() {
        (Vec::new(), n.params.first().copied())
    } else {
        (n.params.iter().copied().map(Some).collect::<Vec<_>>(), None)
    };
    let types = params
        .iter()
        .flatten()
        .map(|&v| n.terms.var_info(v).ty.clone())
        .collect();
    let binding = Binding {
        params,
        types,
        names: declared.params.iter().map(|(n, _)| n.clone()).collect(),
        result: declared.result.clone(),
        unit,
    };
    entry(name, kind, n.terms, n.body, binding)
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
    let qualified = format!("{}::{}", schema.module(def.module).name, def.name);
    let text = format!("{qualified}({})", names.join(", "));
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
    let normal = norm::adhoc(schema, program, &adhoc).map_err(not_normal)?;
    let [body] = normal.items.as_slice() else {
        return Err(Error::internal(Broken::CallNotOneItem));
    };
    let body = *body;
    let shown: Vec<String> = def
        .params
        .iter()
        .map(|(n, t)| format!("{n}: {}", schema.show_ty(t)))
        .collect();
    let written = match &f.module {
        Some(m) => format!("{}::{}", m.text, f.name.text),
        None => f.name.text.clone(),
    };
    let binding = Binding {
        types: def.params.iter().map(|(_, t)| t.clone()).collect(),
        names: def.params.iter().map(|(n, _)| n.clone()).collect(),
        result: def.result.clone(),
        params: names
            .iter()
            .map(|n| normal.params.get(n).copied())
            .collect(),
        unit: None,
    };
    let mut e = entry(&f.name.text, Callable::Fn, normal.terms, body, binding)?;
    e.row.signature = format!("({}) -> {}", shown.join(", "), schema.show_ty(&def.result));
    e.row.text = format!("import fn {written}");
    e.function = schema.ids().function_id(id).map(CatFunctionId::raw);
    e.function_name = Some(qualified);
    Ok(e)
}

/// Returns what a normaliser's refusal of a registered body answers.
fn not_normal(e: norm::Error) -> Error {
    if e.past_limit() {
        Error::invalid(e.message)
    } else {
        Error::internal(Broken::CallNotNormal(e))
    }
}
