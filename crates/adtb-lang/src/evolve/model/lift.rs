use std::collections::BTreeMap;

use super::slots::{Found, default_refs, implied_key, type_slots};
use super::*;
use crate::ast::{self, Decl as AstDecl, RelationClause};
use crate::canon::Names;
use crate::ids::Slot;
use crate::schema::{CtorRef, Schema, ValueDef};
use crate::span::{FileId, Span};
use crate::types::*;

/// Returns the model of `src`, each object taking the id `src` gives it.
pub(super) fn source(src: &Source) -> Result<Model, String> {
    let show = |ds: Vec<crate::span::FileDiag>| {
        ds.into_iter()
            .map(|d| d.diag.message)
            .collect::<Vec<_>>()
            .join("; ")
    };
    let schema = src.schema().map_err(show)?;
    let names = src.names().map_err(show)?;
    let files = src.files().map_err(show)?;
    let mut m = Model {
        modules: src.modules.clone(),
        installed: src.installed.clone(),
        states: src
            .states
            .iter()
            .map(|(&(r, c), &s)| ((r.raw(), c.raw()), s))
            .collect(),
        value_states: src
            .value_states
            .iter()
            .map(|(&c, &s)| (c.raw(), s))
            .collect(),
        grants: src.grants.clone(),
        next: src.next.clone(),
        version: src.version,
        log: src.log.iter().map(LogM::of).collect(),
        logged: src.log.clone(),
        packages: src.packages.clone(),
        ..Model::default()
    };
    for (file, sf) in &files {
        let text = src.file_text(*file);
        for d in &sf.decls {
            let at = Lifting::new(
                &schema,
                *file,
                &text,
                &names[file.0 as usize],
                &[],
                &sf.module.text,
            );
            at.decl(&mut m, d)?;
        }
    }
    Ok(m)
}

/// One declaration being lifted from a built schema whose ids are the model's.
pub(super) struct Lifting<'a> {
    schema: &'a Schema,
    file: FileId,
    /// The file's text, module header included.
    text: &'a str,
    names: &'a Names,
    /// Each name of a relation variable an operator of the migration being applied adds, by the
    /// operator.
    ahead: &'a [(std::ops::Range<usize>, Adder)],
    module: &'a str,
}

fn missing(what: &str) -> String {
    format!("the built schema holds no id for {what}")
}

impl<'a> Lifting<'a> {
    pub(super) fn new(
        schema: &'a Schema,
        file: FileId,
        text: &'a str,
        names: &'a Names,
        ahead: &'a [(std::ops::Range<usize>, Adder)],
        module: &'a str,
    ) -> Lifting<'a> {
        Lifting {
            schema,
            file,
            text,
            names,
            ahead,
            module,
        }
    }

    fn term(&self, span: Span) -> Term {
        Term::lift(self.text, span.range(), self.names, self.ahead)
    }

    /// Returns whether the token before `span` is `(`.
    fn after_paren(&self, span: Span) -> bool {
        let (toks, _) = crate::lex::lex(&self.text[..span.start as usize]);
        toks.iter()
            .rev()
            .find(|t| t.tok != crate::lex::Tok::Eof)
            .is_some_and(|t| t.tok == crate::lex::Tok::LParen)
    }

    fn id(&self, slot: Slot) -> Result<u64, String> {
        let ids = self.schema.ids();
        let id = match &slot {
            Slot::Type(t) => ids.type_id(*t).map(u64::from),
            Slot::Ctor(c) => ids.ctor_id(*c).map(u64::from),
            Slot::Relation(r) => ids.relation_id(*r).map(u64::from),
            Slot::Field(f) => ids.field_id(*f).map(u64::from),
            Slot::Set(r, n) => ids.set_id(*r, n).map(u64::from),
            Slot::Key(r) => ids.key_id(*r).map(u64::from),
            Slot::Index(r, at) => ids.index_id(*r, *at).map(u64::from),
            Slot::Generated(r) => ids.generated_id(*r).map(u64::from),
            Slot::Computed(r, at) => ids.computed_id(*r, *at).map(u64::from),
            Slot::Function(f) => ids.function_id(*f).map(u64::from),
            Slot::Aggregate(a) => ids.aggregate_id(*a).map(u64::from),
            Slot::Constraint(c) => ids.constraint_id(ConstraintIdx::at(*c)).map(u64::from),
            Slot::Check(c) => ids.check_id(CheckIdx::at(*c)).map(u64::from),
            Slot::View(f) => ids.view_id(*f).map(u64::from),
            Slot::ViewKey(f) => ids.view_key_id(*f).map(u64::from),
            Slot::ViewType(f) => ids.view_type_id(*f).map(u64::from),
            Slot::ViewCtor(f) => ids.view_ctor_id(*f).map(u64::from),
            Slot::ViewField(f, n) => ids.view_field_id(*f, n).map(u64::from),
            Slot::Implied(r, f) => ids.implied_id(*r, *f).map(u64::from),
            Slot::Default(..) | Slot::ViewIndex(..) => None,
        };
        id.ok_or_else(|| missing(&format!("{slot:?}")))
    }

    /// Add declaration `d` to `m`, after its others.
    pub(super) fn decl(&self, m: &mut Model, d: &AstDecl) -> Result<(), String> {
        let found = Found::new(self.schema, self.file);
        let item = match d {
            AstDecl::Type(t) => Item::Type(self.ty(m, &found, t)?),
            AstDecl::Relation(r) => Item::Relation(self.relation(m, &found, r)?),
            AstDecl::Set(s) => Item::Set(self.set(m, s)?),
            AstDecl::Fn(f) => {
                let id = self.id(Slot::Function(
                    found.function().ok_or_else(|| missing("a function"))?,
                ))?;
                m.functions.insert(id, self.body(&f.name, f.span));
                Item::Fn(id)
            }
            AstDecl::Aggregate(a) => {
                let id = self.id(Slot::Aggregate(
                    found.aggregate().ok_or_else(|| missing("an aggregate"))?,
                ))?;
                m.functions.insert(id, self.body(&a.name, a.span));
                Item::Aggregate(id)
            }
            AstDecl::Constraint(c) => {
                let id = self.id(Slot::Constraint(
                    found.constraint().ok_or_else(|| missing("a constraint"))?,
                ))?;
                m.denials.insert(id, self.body(&c.name, c.span));
                Item::Constraint(id)
            }
            AstDecl::Check(c) => {
                let id = self.id(Slot::Check(
                    found.check().ok_or_else(|| missing("a check"))?,
                ))?;
                m.denials.insert(id, self.body(&c.name, c.span));
                Item::Check(id)
            }
            AstDecl::Store(s) => Item::Store(self.store(m, s)?),
        };
        m.decls.push(Decl {
            module: self.module.to_string(),
            item,
        });
        Ok(())
    }

    pub(super) fn body(&self, name: &ast::Ident, span: Span) -> Body {
        Body {
            name: name.text.clone(),
            rest: self.term(Span::new(name.span.end as usize, span.end as usize)),
        }
    }

    fn ty(&self, m: &mut Model, found: &Found, t: &ast::TypeDecl) -> Result<u64, String> {
        let def = found.ty().ok_or_else(|| missing("a type"))?;
        let ids = type_slots(self.schema, def)
            .into_iter()
            .map(|s| self.id(s))
            .collect::<Result<Vec<u64>, String>>()?;
        let ty = ids[0];
        let n_ctors = self.schema.ty(def).ctor_refs().count();
        let ctor_ids = &ids[1..=n_ctors];
        let mut field_ids = ids[1 + n_ctors..].iter().copied();
        let mut fields = |owner: u64, fs: &[ast::FieldType], m: &mut Model| {
            let mut out = Vec::new();
            for f in fs {
                let id = field_ids.next().ok_or_else(|| missing("a field"))?;
                m.fields.insert(
                    id,
                    FieldM {
                        owner,
                        name: f.name.text.clone(),
                        ty: self.term(f.ty.span),
                        action: f.on_delete.as_ref().map(|a| self.term(a.span)),
                    },
                );
                out.push(id);
            }
            Ok::<_, String>(out)
        };
        let header = match &t.header {
            Some(h) => Some(fields(ty, h, m)?),
            None => None,
        };
        let body = match &t.body {
            ast::TypeBody::New(base, domain) => {
                m.ctors.insert(
                    ctor_ids[0],
                    CtorM {
                        ty,
                        name: t.name.text.clone(),
                        fields: None,
                    },
                );
                TypeBody::New {
                    ctor: ctor_ids[0],
                    base: self.term(base.span),
                    domain: domain.as_ref().map(|e| self.term(e.span)),
                }
            }
            ast::TypeBody::Record(fs) => {
                let own = fields(ctor_ids[0], fs, m)?;
                m.ctors.insert(
                    ctor_ids[0],
                    CtorM {
                        ty,
                        name: t.name.text.clone(),
                        fields: Some(own),
                    },
                );
                TypeBody::Record { ctor: ctor_ids[0] }
            }
            ast::TypeBody::Ctors(cs) => {
                for (c, &id) in cs.iter().zip(ctor_ids) {
                    let own = match &c.record {
                        Some(fs) => Some(fields(id, fs, m)?),
                        None => None,
                    };
                    m.ctors.insert(
                        id,
                        CtorM {
                            ty,
                            name: c.name.text.clone(),
                            fields: own,
                        },
                    );
                }
                TypeBody::Ctors(ctor_ids.to_vec())
            }
        };
        m.types.insert(
            ty,
            TypeM {
                name: t.name.text.clone(),
                params: t.params.iter().map(|p| p.text.clone()).collect(),
                ordered: t.ordered,
                header,
                body,
            },
        );
        Ok(ty)
    }

    /// Returns a name of the fields `name` of type `def` reaches: the header's, else that field
    /// of constructor `ctor`, else of each constructor.
    fn field_name(&self, def: TypeId, ctor: Option<CtorRef>, name: &ast::Ident) -> Term {
        let td = self.schema.ty(def);
        let refs: Vec<_> = match td.field(None, &name.text) {
            Some(f) => vec![f],
            None => td
                .ctor_refs()
                .filter(|c| ctor.is_none_or(|k| k == *c))
                .filter_map(|c| td.field(Some(c), &name.text))
                .collect(),
        };
        let ids: Vec<Obj> = refs
            .into_iter()
            .filter_map(|f| self.schema.ids().field_id(f))
            .map(|f| (Space::Field, f.raw()))
            .collect();
        Term::names(ids, &name.text)
    }

    /// Returns a name of the constructor or named set `name` of relation `rel`.
    fn scope_name(&self, rel: RelId, name: &ast::Ident) -> Term {
        let ids = self.schema.ids();
        if let Some(s) = ids.set_id(rel, &name.text) {
            return Term::name((Space::Set, s.raw()), &name.text);
        }
        let td = self.schema.ty(self.schema.relation(rel).elem_def);
        let found = td
            .ctor_refs()
            .find(|&c| td.ctor_def(c).name == name.text)
            .and_then(|c| ids.ctor_id(c));
        match found {
            Some(c) => Term::name((Space::Shared, c.raw()), &name.text),
            None => Term::text(&name.text),
        }
    }

    fn relation(&self, m: &mut Model, found: &Found, r: &ast::RelationDecl) -> Result<u64, String> {
        let rel = found
            .relation()
            .ok_or_else(|| missing("a relation variable"))?;
        let id = self.id(Slot::Relation(rel))?;
        let def = self.schema.relation(rel).elem_def;
        let mut clauses = Vec::new();
        let (mut ix, mut dx, mut cx) = (0, 0, 0);
        for c in &r.clauses {
            clauses.push(match c {
                RelationClause::Key(fs, generate) => ClauseM::Key {
                    id: Some(self.id(Slot::Key(rel))?),
                    fields: fs.iter().map(|f| self.field_name(def, None, f)).collect(),
                    listed: self.after_paren(fs[0].span),
                    generate: match generate {
                        Some(g) => Some((g.text.clone(), Some(self.id(Slot::Generated(rel))?))),
                        None => None,
                    },
                },
                RelationClause::Volatile => ClauseM::Volatile,
                RelationClause::Unique(fields, scope, _)
                | RelationClause::Index { fields, scope, .. } => {
                    let kind = match c {
                        RelationClause::Unique(..) => IndexKind::Unique,
                        RelationClause::Index { interval: true, .. } => IndexKind::Interval,
                        _ => IndexKind::Plain,
                    };
                    let clause = ClauseM::Index {
                        was: Vec::new(),
                        id: Some(
                            self.schema
                                .ids()
                                .index_id(rel, ix)
                                .ok_or_else(|| missing("an index"))?
                                .raw(),
                        ),
                        kind,
                        elems: fields.iter().map(|e| self.term(e.span)).collect(),
                        scope: scope.as_ref().map(|s| match s {
                            ast::CtorScope::One(c) => Scope {
                                parts: vec![self.scope_name(rel, c)],
                                listed: false,
                            },
                            ast::CtorScope::Many(cs) => Scope {
                                parts: cs.iter().map(|c| self.scope_name(rel, c)).collect(),
                                listed: true,
                            },
                        }),
                    };
                    ix += 1;
                    clause
                }
                RelationClause::Default { ctor, field, value } => {
                    let td = self.schema.ty(def);
                    let cref = ctor
                        .as_ref()
                        .and_then(|c| td.ctor_refs().find(|&k| td.ctor_def(k).name == c.text));
                    let ids = default_refs(self.schema, rel, dx)
                        .into_iter()
                        .filter_map(|f| {
                            let fid = self.schema.ids().field_id(f)?.raw();
                            let did = self
                                .schema
                                .ids()
                                .clause_default_ids(rel, dx)
                                .find(|(x, _)| *x == f)?
                                .1
                                .raw();
                            Some((fid, did))
                        })
                        .collect();
                    let clause = ClauseM::Default {
                        ctor: ctor.as_ref().map(|c| {
                            match cref.and_then(|k| self.schema.ids().ctor_id(k)) {
                                Some(k) => Term::name((Space::Shared, k.raw()), &c.text),
                                None => Term::text(&c.text),
                            }
                        }),
                        field: self.field_name(def, cref, field),
                        value: match value {
                            ast::DefaultValue::Generate(g) => {
                                Term::text(&format!("generate {}", g.text))
                            }
                            ast::DefaultValue::Expr(e) => self.term(e.span),
                        },
                        ids,
                    };
                    dx += 1;
                    clause
                }
                RelationClause::Computed(n, ty, e) => {
                    let clause = ClauseM::Computed {
                        id: Some(self.id(Slot::Computed(rel, cx))?),
                        name: n.text.clone(),
                        ty: self.term(ty.span),
                        expr: self.term(e.span),
                    };
                    cx += 1;
                    clause
                }
            });
        }
        let mut implied = BTreeMap::new();
        for i in self.schema.implied_indexes(rel) {
            let f = i.field_ref();
            if let (Some(key), Ok(index)) = (
                implied_key(self.schema, &i, |f| {
                    self.schema.ids().field_id(*f).map(u64::from)
                }),
                self.id(Slot::Implied(rel, f)),
            ) {
                implied.insert(key, index);
            }
        }
        m.relations.insert(
            id,
            RelM {
                name: r.name.name.text.clone(),
                ty: self.term(r.ty.span),
                clauses,
                implied,
            },
        );
        Ok(id)
    }

    fn value(&self, name: &str) -> Option<ValueDef> {
        let mid = self.schema.module_named(self.module)?;
        self.schema.module(mid).values.get(name).copied()
    }

    fn set(&self, m: &mut Model, s: &ast::SetDecl) -> Result<u64, String> {
        let Some(ValueDef::Relation(rel)) = self.value(&s.relation.text) else {
            return Err(missing("a named set's relation variable"));
        };
        let id = self.id(Slot::Set(rel, s.name.text.clone()))?;
        let relation = self.id(Slot::Relation(rel))?;
        m.sets.insert(
            id,
            SetM {
                relation,
                name: s.name.text.clone(),
                members: s.members.iter().map(|c| self.scope_name(rel, c)).collect(),
            },
        );
        Ok(id)
    }

    fn store(&self, m: &mut Model, s: &ast::StoreDecl) -> Result<u64, String> {
        let Some(ValueDef::Function(f)) = self.value(&s.function()?.text) else {
            return Err(missing("a stored view's function"));
        };
        let view = self.id(Slot::View(f))?;
        let function = self.id(Slot::Function(f))?;
        let ids = self.schema.ids();
        let mut held = BTreeMap::new();
        if let Some(k) = ids.view_key_id(f) {
            held.insert(ViewSlot::Key, k.raw());
        }
        if let Some(t) = ids.view_type_id(f) {
            held.insert(ViewSlot::Type, t.raw());
        }
        if let Some(c) = ids.view_ctor_id(f) {
            held.insert(ViewSlot::Ctor, c.raw());
        }
        if let Ty::Set(elem) = &self.schema.function(f).result
            && let Ty::Record(fs) = &**elem
        {
            for (name, _) in fs {
                if let Some(x) = ids.view_field_id(f, name) {
                    held.insert(ViewSlot::Field(name.clone()), x.raw());
                }
            }
        }
        let n = s
            .clauses
            .iter()
            .filter(|c| matches!(c, RelationClause::Unique(..) | RelationClause::Index { .. }))
            .count();
        for k in 0..n {
            if let Some(x) = ids.view_index_id(f, k) {
                held.insert(ViewSlot::Index(k), x.raw());
            }
        }
        m.stores.insert(
            view,
            StoreM {
                function,
                written: s.written(),
                rest: self.term(Span::new(s.view.span().end as usize, s.span.end as usize)),
                ids: held,
            },
        );
        Ok(view)
    }
}
