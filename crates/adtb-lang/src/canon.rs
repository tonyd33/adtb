use std::collections::BTreeMap;
use std::ops::Range;

use crate::ast::{self, CtorScope, CtorSel, Decl, RelationClause, TypeBody};
use crate::check::Ref;
use crate::lex::{Tok, decode_text, lex};
use crate::schema::{PRELUDE, Schema, TypeName, ValueDef};
use crate::span::Span;
use crate::types::{ModuleId, RelId, Ty, TypeId};

#[cfg(test)]
#[path = "canon_tests.rs"]
mod tests;

/// The module each schema object name of a text resolves to, by the span of the name's identifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    /// `None` for a name of the prelude.
    modules: BTreeMap<(u32, u32), Option<String>>,
}

impl Names {
    /// Records that the identifier at `span` names an object of `module`.
    pub fn qualify(&mut self, schema: &Schema, span: Span, module: ModuleId) {
        let name = (module != PRELUDE).then(|| schema.module(module).name.clone());
        self.modules.insert((span.start, span.end), name);
    }

    /// Records the object a checked body names at `span`; a field or a `match` is no name of an object.
    pub fn note(&mut self, schema: &Schema, span: Span, r: &Ref) {
        let module = match r {
            Ref::Type(d) | Ref::Ctor(d, _) => schema.ty(*d).module,
            Ref::Relation(r) | Ref::Set(r, _) => schema.relation(*r).module,
            Ref::Function(f) => schema.function(*f).module,
            Ref::Aggregate(a) => schema.aggregates[*a as usize].module,
            Ref::Field { .. } | Ref::Match(_) => return,
        };
        self.qualify(schema, span, module);
    }

    /// Returns the module the identifier at `span` names an object of, `Some(None)` for the prelude.
    pub fn get(&self, span: Span) -> Option<Option<&str>> {
        self.modules
            .get(&(span.start, span.end))
            .map(|m| m.as_deref())
    }

    /// Copies the names of `from` inside `range`, moved so that `range.start` lands at `to`.
    pub fn copy(&mut self, from: &Names, range: Range<usize>, to: usize) {
        let (lo, hi) = (range.start as u32, range.end as u32);
        for (&(s, e), m) in from.modules.range((lo, lo)..(hi, hi)) {
            if e <= hi {
                let at = |x: u32| (x - lo) + to as u32;
                self.modules.insert((at(s), at(e)), m.clone());
            }
        }
    }

    pub fn extend(&mut self, other: Names) {
        self.modules.extend(other.modules);
    }
}

/// Returns the canonical token form of the term at `span` of `text`, its names spelled as `names` resolves them.
///
/// Preconditions: `span` lies on token boundaries of `text`, and `names` holds every schema object name in it.
pub fn canonical_term(text: &str, span: Span, names: &Names) -> String {
    let base = span.start;
    let (tokens, _) = lex(&text[span.range()]);
    let tokens: Vec<(Tok, Span, &str)> = tokens
        .iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| {
            let at = Span {
                start: t.span.start + base,
                end: t.span.end + base,
            };
            (t.tok, at, &text[at.range()])
        })
        .collect();
    let ident = |t: Tok| matches!(t, Tok::Lower | Tok::Upper);
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let (tok, at, written) = tokens[i];
        let qualified = ident(tok)
            && tokens.get(i + 1).is_some_and(|t| t.0 == Tok::ColonColon)
            && tokens
                .get(i + 2)
                .is_some_and(|t| ident(t.0) && names.get(t.1).is_some());
        if qualified {
            let (_, at, written) = tokens[i + 2];
            out.push(spelled(names, at, written));
            i += 3;
            continue;
        }
        out.push(match tok {
            Tok::Lower | Tok::Upper => spelled(names, at, written),
            Tok::Int => digits(written).to_string(),
            Tok::Decimal => decimal(written),
            Tok::Float => float(written),
            Tok::Duration => {
                let end = written
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(written.len());
                format!("{}{}", digits(&written[..end]), &written[end..])
            }
            Tok::Text => text_literal(&decode_text(written)),
            _ => written.to_string(),
        });
        i += 1;
    }
    out.join(" ")
}

fn spelled(names: &Names, at: Span, written: &str) -> String {
    match names.get(at) {
        Some(Some(module)) => format!("{module}::{written}"),
        _ => written.to_string(),
    }
}

/// Returns `ds` without leading zeros, `0` for zero.
fn digits(ds: &str) -> &str {
    match ds.trim_start_matches('0') {
        "" => "0",
        rest => rest,
    }
}

fn decimal(written: &str) -> String {
    let (int, frac) = written.split_once('.').unwrap_or((written, ""));
    format!("{}.{frac}", digits(int))
}

fn float(written: &str) -> String {
    let (mantissa, exp) = written
        .split_once(['e', 'E'])
        .expect("a float literal has an exponent");
    let (int, frac) = match mantissa.split_once('.') {
        Some((int, frac)) => (int, format!(".{frac}")),
        None => (mantissa, String::new()),
    };
    let (sign, exp) = match exp.as_bytes().first() {
        Some(b'-') => ("-", &exp[1..]),
        Some(b'+') => ("", &exp[1..]),
        _ => ("", exp),
    };
    format!("{}{frac}e{sign}{}", digits(int), digits(exp))
}

fn text_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Appends the schema objects type expression `t` names, seen from `module`.
pub fn type_refs(schema: &Schema, module: ModuleId, t: &ast::Type, out: &mut Vec<(Span, Ref)>) {
    match &t.kind {
        ast::TypeKind::Named { name, sel, args } => {
            if let Ok(TypeName::Decl(def)) = schema.lookup_type(module, name) {
                out.push((name.name.span, Ref::Type(def)));
                if let Some(sel) = sel {
                    type_sel_refs(schema, def, sel, out);
                }
            }
            for a in args {
                if let ast::TypeArg::Type(t) = a {
                    type_refs(schema, module, t, out);
                }
            }
        }
        ast::TypeKind::Key(r) => relation_ref_refs(schema, module, r, out),
        ast::TypeKind::Record(fs) => fs
            .iter()
            .for_each(|f| type_refs(schema, module, &f.ty, out)),
        ast::TypeKind::Sum(cs) => cs
            .iter()
            .flat_map(|c| c.record.iter().flatten())
            .for_each(|f| type_refs(schema, module, &f.ty, out)),
        ast::TypeKind::Fn(a, b) => {
            type_refs(schema, module, a, out);
            type_refs(schema, module, b, out);
        }
        ast::TypeKind::Var(_) | ast::TypeKind::Unit => {}
    }
}

fn sel_idents(sel: &CtorSel) -> &[ast::Ident] {
    match sel {
        CtorSel::One(c) => std::slice::from_ref(c),
        CtorSel::Many(cs) => cs,
    }
}

fn scope_idents(s: &CtorScope) -> &[ast::Ident] {
    match s {
        CtorScope::One(c) => std::slice::from_ref(c),
        CtorScope::Many(cs) => cs,
    }
}

/// Appends the constructors and named sets a type-level selection over `def` names.
fn type_sel_refs(schema: &Schema, def: TypeId, sel: &CtorSel, out: &mut Vec<(Span, Ref)>) {
    for c in sel_idents(sel) {
        if let Some(i) = schema.ty(def).ctor_index(&c.text) {
            out.push((c.span, Ref::Ctor(def, i)));
        } else if let Some(r) = schema
            .relations
            .iter()
            .position(|r| r.elem_def == def && r.sets.iter().any(|s| s.name == c.text))
        {
            out.push((c.span, Ref::Set(RelId(r as u32), c.text.clone())));
        }
    }
}

/// Appends the constructors and named sets of relation `rel` the identifiers name.
fn rel_sel_refs(schema: &Schema, rel: RelId, names: &[ast::Ident], out: &mut Vec<(Span, Ref)>) {
    let r = schema.relation(rel);
    for c in names {
        if let Some(i) = schema.ty(r.elem_def).ctor_index(&c.text) {
            out.push((c.span, Ref::Ctor(r.elem_def, i)));
        } else if r.sets.iter().any(|s| s.name == c.text) {
            out.push((c.span, Ref::Set(rel, c.text.clone())));
        }
    }
}

fn relation_ref_refs(
    schema: &Schema,
    module: ModuleId,
    r: &ast::RelationRef,
    out: &mut Vec<(Span, Ref)>,
) {
    let Ok(rel) = schema.lookup_relation(module, &r.relation) else {
        return;
    };
    out.push((r.relation.name.span, Ref::Relation(rel)));
    if let Some(sel) = &r.sel {
        rel_sel_refs(schema, rel, sel_idents(sel), out);
    }
}

/// Records the schema objects a declaration of `module` declares and names outside its bodies.
///
/// The names inside bodies (function bodies, constraints, checks, domains, defaults, computed and index
/// fields, retag records and aggregate parts) are the checker's, as [`crate::check::schema_refs`] reports them.
pub fn decl_names(schema: &Schema, module: ModuleId, d: &Decl, names: &mut Names) {
    let mut refs = Vec::new();
    let own = |names: &mut Names, span: Span| names.qualify(schema, span, module);
    let tys = |ts: &mut dyn Iterator<Item = &ast::Type>, refs: &mut Vec<(Span, Ref)>| {
        ts.for_each(|t| type_refs(schema, module, t, refs))
    };
    match d {
        Decl::Type(t) => {
            own(names, t.name.span);
            let def = schema.module(module).types.get(&t.name.text).copied();
            let mut fields: Vec<&ast::FieldType> = t.header.iter().flatten().collect();
            match &t.body {
                TypeBody::New(base, _) => tys(&mut std::iter::once(base), &mut refs),
                TypeBody::Record(fs) => fields.extend(fs),
                TypeBody::Ctors(cs) => {
                    for c in cs {
                        own(names, c.name.span);
                        fields.extend(c.record.iter().flatten());
                    }
                }
            }
            tys(&mut fields.iter().map(|f| &f.ty), &mut refs);
            for f in &fields {
                if let (Some(ast::RefAction::Retag(c, _)), Some(def)) = (&f.on_delete, def)
                    && let Some(i) = schema.ty(def).ctor_index(&c.text)
                {
                    refs.push((c.span, Ref::Ctor(def, i)));
                }
            }
        }
        Decl::Relation(r) => {
            own(names, r.name.name.span);
            tys(&mut std::iter::once(&r.ty), &mut refs);
            if let Ok(rel) = schema.lookup_relation(module, &r.name) {
                clause_refs(schema, module, rel, &r.clauses, &mut refs);
            }
        }
        Decl::Set(s) => {
            own(names, s.name.span);
            let q = ast::QName {
                module: None,
                name: s.relation.clone(),
            };
            if let Ok(rel) = schema.lookup_relation(module, &q) {
                refs.push((s.relation.span, Ref::Relation(rel)));
                rel_sel_refs(schema, rel, &s.members, &mut refs);
            }
        }
        Decl::Fn(f) => {
            own(names, f.name.span);
            tys(
                &mut f.params.iter().map(|p| &p.ty).chain([&f.result]),
                &mut refs,
            );
        }
        Decl::Constraint(c) => own(names, c.name.span),
        Decl::Check(c) => {
            own(names, c.name.span);
            relation_ref_refs(schema, module, &c.on, &mut refs);
        }
        Decl::Store(s) => {
            if let Ok(ValueDef::Function(f)) = schema.lookup_value(module, &s.view) {
                refs.push((s.view.name.span, Ref::Function(f)));
                if let Ty::Set(elem) = &schema.function(f).result
                    && let Ty::Data { def, .. } = elem.as_ref()
                {
                    for c in &s.clauses {
                        if let Some(scope) = clause_scope(c) {
                            for n in scope_idents(scope) {
                                if let Some(i) = schema.ty(*def).ctor_index(&n.text) {
                                    refs.push((n.span, Ref::Ctor(*def, i)));
                                }
                            }
                        }
                    }
                }
            }
        }
        Decl::Aggregate(a) => {
            own(names, a.name.span);
            tys(
                &mut a
                    .params
                    .iter()
                    .map(|p| &p.ty)
                    .chain([&a.result])
                    .chain(a.state.iter().map(|f| &f.ty)),
                &mut refs,
            );
        }
    }
    for (span, r) in &refs {
        names.note(schema, *span, r);
    }
}

fn clause_scope(c: &RelationClause) -> Option<&CtorScope> {
    match c {
        RelationClause::Unique(_, scope) | RelationClause::Index { scope, .. } => scope.as_ref(),
        _ => None,
    }
}

fn clause_refs(
    schema: &Schema,
    module: ModuleId,
    rel: RelId,
    clauses: &[RelationClause],
    out: &mut Vec<(Span, Ref)>,
) {
    for c in clauses {
        if let Some(scope) = clause_scope(c) {
            rel_sel_refs(schema, rel, scope_idents(scope), out);
        }
        match c {
            RelationClause::Default { ctor: Some(c), .. } => {
                rel_sel_refs(schema, rel, std::slice::from_ref(c), out)
            }
            RelationClause::Computed(_, t, _) => type_refs(schema, module, t, out),
            _ => {}
        }
    }
}
