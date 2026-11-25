//! Print terms in their canonical form, with names qualified.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;

use crate::ast::{self, CtorScope, CtorSel, Decl, RelationClause, TypeBody};
use crate::check::Ref;
use crate::ids::Space;
use crate::lex::{Tok, decode_text, lex};
use crate::schema::{PRELUDE, Schema, TypeName, ValueDef};
use crate::span::{Diag, Span};
use crate::types::{ModuleId, RelId, Ty, TypeId};

#[cfg(test)]
#[path = "canon_tests.rs"]
mod tests;

/// The module each schema object name of a text resolves to, by the span of the name's identifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    /// `None` for a name written unqualified: of the prelude, or of an anonymous constructor.
    modules: BTreeMap<(u32, u32), Option<String>>,
    /// The catalog ids of the objects each name of a user module resolves to, fields included: a
    /// field name of a grant resolves to the field of each constructor it covers.
    ids: BTreeMap<(u32, u32), BTreeSet<(Space, u64)>>,
    /// The field names written as a pun, standing for the field and a variable of its name.
    puns: BTreeSet<(u32, u32)>,
    /// The names of a type, relation variable, constructor or named set the schema does not declare.
    unresolved: BTreeSet<(u32, u32)>,
}

impl Names {
    /// Record that the identifier at `span` names an object of `module`.
    pub fn qualify(&mut self, schema: &Schema, span: Span, module: ModuleId) {
        let name = (module != PRELUDE).then(|| schema.module(module).name.clone());
        self.modules.insert((span.start, span.end), name);
    }

    /// Record the object a checked body names at `span`: its catalog id, and its module unless it
    /// is a field or a `match`.
    pub fn note(&mut self, schema: &Schema, span: Span, r: &Ref) {
        if let Some(id) = catalog_id(schema, r) {
            self.ids
                .entry((span.start, span.end))
                .or_default()
                .insert(id);
        }
        if let Ref::Field { pun: true, .. } = r {
            self.puns.insert((span.start, span.end));
        }
        if let Ref::Unresolved = r {
            self.unresolved.insert((span.start, span.end));
        }
        let module = match r {
            Ref::Type(d) | Ref::Ctor(d, _) => schema.ty(*d).module,
            Ref::Relation(r) | Ref::Set(r, _) => schema.relation(*r).module,
            Ref::Function(f) => schema.function(*f).module,
            Ref::Aggregate(a) => schema.aggregates[*a as usize].module,
            Ref::BuiltinType | Ref::Anonymous => PRELUDE,
            Ref::Field { .. } | Ref::Match(_) | Ref::Unresolved => return,
        };
        self.qualify(schema, span, module);
    }

    /// Returns the module the identifier at `span` names an object of, `Some(None)` for the prelude.
    pub fn get(&self, span: Span) -> Option<Option<&str>> {
        self.modules
            .get(&(span.start, span.end))
            .map(|m| m.as_deref())
    }

    /// Returns whether every name of a type, relation variable, constructor or named set resolves.
    pub fn all_resolve(&self) -> bool {
        self.unresolved.is_empty()
    }

    /// Returns the catalog id of each object the names resolve to.
    pub fn ids(&self) -> impl Iterator<Item = (Space, u64)> + '_ {
        self.ids.values().flatten().copied()
    }

    /// Returns the span of each name.
    pub fn spans(&self) -> impl Iterator<Item = Span> + '_ {
        self.modules
            .keys()
            .chain(self.ids.keys())
            .chain(&self.puns)
            .chain(&self.unresolved)
            .map(|&(start, end)| Span::new(start as usize, end as usize))
    }

    /// Record that the identifier at `span` names the object of catalog id `id`.
    pub fn tie(&mut self, span: Span, id: (Space, u64)) {
        self.ids
            .entry((span.start, span.end))
            .or_default()
            .insert(id);
    }

    /// Returns each name inside `range` that resolves to a catalog object, by its range, with the
    /// ids it resolves to and whether it is a pun, in text order.
    pub fn ids_within(
        &self,
        range: Range<usize>,
    ) -> impl Iterator<Item = (Range<usize>, &BTreeSet<(Space, u64)>, bool)> + '_ {
        let (lo, hi) = (range.start as u32, range.end as u32);
        self.ids
            .range((lo, lo)..(hi, hi))
            .filter(move |((_, e), _)| *e <= hi)
            .map(|(&(s, e), ids)| (s as usize..e as usize, ids, self.puns.contains(&(s, e))))
    }

    /// Copy the names of `from` inside `range`, moved so that `range.start` lands at `to`.
    pub fn copy(&mut self, from: &Names, range: Range<usize>, to: usize) {
        let (lo, hi) = (range.start as u32, range.end as u32);
        let at = |x: u32| (x - lo) + to as u32;
        for (&(s, e), m) in from.modules.range((lo, lo)..(hi, hi)) {
            if e <= hi {
                self.modules.insert((at(s), at(e)), m.clone());
            }
        }
        for (&(s, e), ids) in from.ids.range((lo, lo)..(hi, hi)) {
            if e <= hi {
                self.ids
                    .entry((at(s), at(e)))
                    .or_default()
                    .extend(ids.iter().copied());
            }
        }
        for &(s, e) in from.puns.range((lo, lo)..(hi, hi)) {
            if e <= hi {
                self.puns.insert((at(s), at(e)));
            }
        }
    }

    /// Copy the names of `from` inside `range` as `copy` does, except at each place that already
    /// names an object.
    pub fn copy_unnamed(&mut self, from: &Names, range: Range<usize>, to: usize) {
        let mut fresh = Names::default();
        fresh.copy(from, range, to);
        fresh.modules.retain(|span, _| !self.names_at(*span));
        fresh.ids.retain(|span, _| !self.names_at(*span));
        self.extend(fresh);
    }

    /// Record at `to` the objects the names of `from` at each of `spans` resolve to, in the
    /// module of the first.
    pub fn gather(&mut self, from: &Names, spans: &[Range<usize>], to: Range<usize>) {
        let key = |r: &Range<usize>| (r.start as u32, r.end as u32);
        let at = key(&to);
        if let Some(m) = spans.first().and_then(|r| from.modules.get(&key(r))) {
            self.modules.insert(at, m.clone());
        }
        for r in spans {
            if let Some(ids) = from.ids.get(&key(r)) {
                self.ids.entry(at).or_default().extend(ids.iter().copied());
            }
        }
    }

    fn names_at(&self, span: (u32, u32)) -> bool {
        self.modules.contains_key(&span) || self.ids.contains_key(&span)
    }

    pub fn extend(&mut self, other: Names) {
        self.modules.extend(other.modules);
        for (span, ids) in other.ids {
            self.ids.entry(span).or_default().extend(ids);
        }
        self.puns.extend(other.puns);
        self.unresolved.extend(other.unresolved);
    }
}

/// Returns the catalog id of the object `r` names, `None` for one with no id.
fn catalog_id(schema: &Schema, r: &Ref) -> Option<(Space, u64)> {
    let ids = &schema.ids();
    Some(match r {
        Ref::Type(d) => (Space::Shared, ids.type_id(*d)?.raw()),
        Ref::Ctor(d, c) => (
            Space::Shared,
            ids.ctor_id(schema.ty(*d).ctor_at(*c)?)?.raw(),
        ),
        Ref::Relation(rel) => (Space::Shared, ids.relation_id(*rel)?.raw()),
        Ref::Set(rel, name) => (Space::Set, ids.set_id(*rel, name)?.raw()),
        Ref::Function(f) => (Space::Function, ids.function_id(*f)?.raw()),
        Ref::Aggregate(a) => (Space::Function, ids.aggregate_id(*a as usize)?.raw()),
        Ref::Field {
            def, ctor, name, ..
        } => {
            let td = schema.ty(*def);
            let f = td.field(ctor.and_then(|c| td.ctor_at(c)), name)?;
            (Space::Field, ids.field_id(f)?.raw())
        }
        Ref::BuiltinType | Ref::Anonymous | Ref::Match(_) | Ref::Unresolved => return None,
    })
}

/// A name of a type, constructor or named set, or a qualified name, that a term writes and the
/// names it is spelled by do not resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unnamed {
    at: Span,
    written: String,
}

impl fmt::Display for Unnamed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` resolves to no schema object", self.written)
    }
}

impl From<Unnamed> for Diag {
    fn from(u: Unnamed) -> Diag {
        Diag::new(u.at, format!("internal error: {u}"))
    }
}

/// Returns the canonical token form of the term at `span` of `text`, its names spelled as `names` resolves them.
///
/// Fail with the first name of a type, constructor or named set, or qualified name, that `names`
/// does not hold.
///
/// Preconditions: `span` lies on token boundaries of `text`.
pub fn canonical_term(text: &str, span: Span, names: &Names) -> Result<String, Unnamed> {
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
        if tok == Tok::ColonColon || (tok == Tok::Upper && names.get(at).is_none()) {
            return Err(Unnamed {
                at,
                written: written.to_string(),
            });
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
    Ok(out.join(" "))
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

/// Append the schema objects type expression `t` names, seen from `module`.
pub fn type_refs(schema: &Schema, module: ModuleId, t: &ast::Type, out: &mut Vec<(Span, Ref)>) {
    match &t.kind {
        ast::TypeKind::Named { name, sel, args } => {
            match schema.lookup_type(module, name) {
                Ok(TypeName::Decl(def)) => {
                    out.push((name.name.span, Ref::Type(def)));
                    if let Some(sel) = sel {
                        type_sel_refs(schema, def, sel, out);
                    }
                }
                Ok(_) => out.push((name.name.span, Ref::BuiltinType)),
                Err(_) => out.push((name.name.span, Ref::Unresolved)),
            }
            for a in args {
                if let ast::TypeArg::Type(t) = a {
                    type_refs(schema, module, t, out);
                }
            }
        }
        ast::TypeKind::Key(word, r) => {
            out.push((*word, Ref::BuiltinType));
            relation_ref_refs(schema, module, r, out);
        }
        ast::TypeKind::Record(fs) => fs
            .iter()
            .for_each(|f| type_refs(schema, module, &f.ty, out)),
        ast::TypeKind::Sum(cs) => {
            for c in cs {
                out.push((c.name.span, Ref::Anonymous));
                for f in c.record.iter().flatten() {
                    type_refs(schema, module, &f.ty, out);
                }
            }
        }
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

/// Append the constructors and named sets a type-level selection over `def` names.
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
        } else {
            out.push((c.span, Ref::Unresolved));
        }
    }
}

/// Append the constructors and named sets of relation `rel` the identifiers name.
fn rel_sel_refs(schema: &Schema, rel: RelId, names: &[ast::Ident], out: &mut Vec<(Span, Ref)>) {
    let r = schema.relation(rel);
    for c in names {
        if let Some(i) = schema.ty(r.elem_def).ctor_index(&c.text) {
            out.push((c.span, Ref::Ctor(r.elem_def, i)));
        } else if r.sets.iter().any(|s| s.name == c.text) {
            out.push((c.span, Ref::Set(rel, c.text.clone())));
        } else {
            out.push((c.span, Ref::Unresolved));
        }
    }
}

/// Returns the reference of field `f` of constructor `ctor` of type `def`, else of its header,
/// else of its only constructor when the type is a record.
fn field_ref(
    schema: &Schema,
    def: TypeId,
    ctor: Option<u32>,
    f: &ast::Ident,
) -> Option<(Span, Ref)> {
    let td = schema.ty(def);
    let ctor = match ctor {
        Some(c) => Some(c),
        None if td.field(None, &f.text).is_some() => None,
        None => (td.ctors().len() == 1).then_some(0),
    };
    td.field(ctor.and_then(|c| td.ctor_at(c)), &f.text)?;
    Some((
        f.span,
        Ref::Field {
            def,
            ctor,
            name: f.text.clone(),
            pun: false,
        },
    ))
}

fn relation_ref_refs(
    schema: &Schema,
    module: ModuleId,
    r: &ast::RelationRef,
    out: &mut Vec<(Span, Ref)>,
) {
    let Ok(rel) = schema.lookup_relation(module, &r.relation) else {
        out.push((r.relation.name.span, Ref::Unresolved));
        return;
    };
    out.push((r.relation.name.span, Ref::Relation(rel)));
    if let Some(sel) = &r.sel {
        rel_sel_refs(schema, rel, sel_idents(sel), out);
    }
}

/// Record the schema objects a declaration of `module` declares and names outside its bodies.
///
/// The names inside bodies (function bodies, constraints, checks, domains, defaults, computed and index
/// fields, retag records and aggregate parts) are the checker's, as [`crate::check::schema_refs`] reports them.
pub fn decl_names(schema: &Schema, module: ModuleId, d: &Decl, names: &mut Names) {
    let mut own = |span: Span| names.qualify(schema, span, module);
    match d {
        Decl::Type(t) => {
            own(t.name.span);
            if let TypeBody::Ctors(cs) = &t.body {
                cs.iter().for_each(|c| own(c.name.span));
            }
        }
        Decl::Relation(r) => own(r.name.name.span),
        Decl::Set(s) => own(s.name.span),
        Decl::Fn(f) => {
            own(f.name.span);
            for b in f.type_params.iter().flat_map(|p| &p.bounds) {
                names.qualify(schema, b.span, PRELUDE);
            }
        }
        Decl::Constraint(c) => own(c.name.span),
        Decl::Check(c) => own(c.name.span),
        Decl::Store(_) => {}
        Decl::Aggregate(a) => own(a.name.span),
    }
    for (span, r) in decl_refs(schema, module, d) {
        names.note(schema, span, &r);
    }
}

/// Returns the schema objects a declaration of `module` names outside its bodies, by the span
/// that names each.
pub fn decl_refs(schema: &Schema, module: ModuleId, d: &Decl) -> Vec<(Span, Ref)> {
    let mut refs = Vec::new();
    let tys = |ts: &mut dyn Iterator<Item = &ast::Type>, refs: &mut Vec<(Span, Ref)>| {
        ts.for_each(|t| type_refs(schema, module, t, refs));
    };
    match d {
        Decl::Type(t) => {
            let def = schema.module(module).types.get(&t.name.text).copied();
            let mut fields: Vec<&ast::FieldType> = t.header.iter().flatten().collect();
            match &t.body {
                TypeBody::New(base, _) => tys(&mut std::iter::once(base), &mut refs),
                TypeBody::Record(fs) => fields.extend(fs),
                TypeBody::Ctors(cs) => {
                    for c in cs {
                        fields.extend(c.record.iter().flatten());
                    }
                }
            }
            tys(&mut fields.iter().map(|f| &f.ty), &mut refs);
            for f in &fields {
                if let (Some(ast::RefAction::Retag(c, _)), Some(def)) =
                    (f.on_delete.as_ref().map(|a| &a.action), def)
                    && let Some(i) = schema.ty(def).ctor_index(&c.text)
                {
                    refs.push((c.span, Ref::Ctor(def, i)));
                }
            }
        }
        Decl::Relation(r) => {
            tys(&mut std::iter::once(&r.ty), &mut refs);
            if let Ok(rel) = schema.lookup_relation(module, &r.name) {
                clause_refs(schema, module, rel, &r.clauses, &mut refs);
            }
        }
        Decl::Set(s) => {
            let q = ast::QName {
                module: None,
                name: s.relation.clone(),
            };
            if let Ok(rel) = schema.lookup_relation(module, &q) {
                refs.push((s.relation.span, Ref::Relation(rel)));
                rel_sel_refs(schema, rel, &s.members, &mut refs);
            }
        }
        Decl::Fn(f) => tys(
            &mut f.params.iter().map(|p| &p.ty).chain([&f.result]),
            &mut refs,
        ),
        Decl::Constraint(_) => {}
        Decl::Check(c) => {
            relation_ref_refs(schema, module, &c.on, &mut refs);
        }
        Decl::Store(s) => {
            if let Ok(ValueDef::Function(f)) = schema.lookup_value(module, &s.view) {
                refs.push((s.view.span(), Ref::Function(f)));
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
        Decl::Aggregate(a) => tys(
            &mut a
                .params
                .iter()
                .map(|p| &p.ty)
                .chain([&a.result])
                .chain(a.state.iter().map(|f| &f.ty)),
            &mut refs,
        ),
    }
    refs
}

fn clause_scope(c: &RelationClause) -> Option<&CtorScope> {
    match c {
        RelationClause::Unique(_, scope, _) | RelationClause::Index { scope, .. } => scope.as_ref(),
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
        let def = schema.relation(rel).elem_def;
        match c {
            RelationClause::Key(fields, _) => {
                for f in fields {
                    out.extend(field_ref(schema, def, None, f));
                }
            }
            RelationClause::Default { ctor, field, .. } => {
                if let Some(c) = ctor {
                    rel_sel_refs(schema, rel, std::slice::from_ref(c), out);
                }
                let at = ctor
                    .as_ref()
                    .and_then(|c| schema.ty(def).ctor_index(&c.text));
                out.extend(field_ref(schema, def, at, field));
            }
            RelationClause::Computed(_, t, _) => type_refs(schema, module, t, out),
            _ => {}
        }
    }
}
