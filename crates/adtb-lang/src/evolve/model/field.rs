use super::op::{self, Fail, Res, broken, refuse, refuse_at, unchecked};
use super::run::{Built, Decls, Late, Run};
use super::*;
use crate::ast::{self, Fill};
use crate::canon::{Names, canonical_term};
use crate::capability::RootKind;
use crate::check::{self, RowBody};
use crate::evolve::{Clause, Resolved, RowFn, RowMap};
use crate::schema::{CtorRef, Schema, TypeDefKind};
use crate::span::{FileId, Span};
use crate::types::{Ty, TypeId};

/// An operator's result: its resolved form, its clauses and its row maps.
/// An operator applied: its resolved form, its clauses, and its row maps or why they fail.
pub(super) type Applied = (Resolved, Vec<Clause>, Maps);

/// The row maps of an operator's one step, or why they cannot be stated.
pub(super) type Maps = Result<Vec<RowMap>, String>;

impl Model {
    /// Returns whether a field named `name` may join `ctor` of type `t`, the header when `None`.
    fn field_free(&self, t: u64, ctor: Option<u64>, name: &str) -> bool {
        !self.fields_of(t).into_iter().any(|f| {
            let x = &self.fields[&f];
            x.name == name
                && (ctor.is_none() || self.types.contains_key(&x.owner) || Some(x.owner) == ctor)
        })
    }

    /// Returns the field of `ctor` of type `t`, the header when `None`, named `name`.
    pub(super) fn field_of(&self, t: u64, ctor: Option<u64>, name: &str) -> Option<u64> {
        self.own_fields(t, ctor)
            .into_iter()
            .find(|f| self.fields[f].name == name)
    }

    /// Returns the schema the model builds now and its program, refusing the operator as one
    /// that checks against a schema that does not check when they do not build or check.
    pub(super) fn checked(&self, run: &Run) -> Res<(Schema, check::Program)> {
        let schema = match self.schema(&run.ahead(self)) {
            Ok(s) => s,
            Err(ds) => return unchecked(&ds),
        };
        match check::check_schema(&schema) {
            Ok(p) => Ok((schema, p)),
            Err(ds) => unchecked(&ds),
        }
    }

    /// Returns the type `t` is in `schema`.
    pub(super) fn def_in(&self, schema: &Schema, t: u64) -> Option<TypeId> {
        schema.ids().type_of(crate::ids::CatTypeId::from_raw(t))
    }

    pub(super) fn ctor_index(&self, schema: &Schema, c: u64) -> Option<u32> {
        schema
            .ids()
            .ctor_of(crate::ids::CatCtorId::from_raw(c))
            .map(CtorRef::index)
    }

    /// Lift the type and delete action of field `f` from `built`, every name in them resolving
    /// as `built` resolves it. Returns the file of its type's declaration.
    fn lift_field(&mut self, built: &Built, f: u64) -> Res<FileId> {
        let t = self.field_type(f).expect("a held field");
        let file = FileId(self.decl_at(Item::Type(t)).expect("a declared type") as u32);
        let owner = self.fields[&f].owner;
        let ctor = self.ctors.get(&owner).map(|c| c.name.clone());
        let written = Self::field_written(built, file, ctor.as_deref(), &self.fields[&f].name)?;
        let names = built.names(file);
        let ahead = built.ahead_ranges(file);
        let lift = |span: Span| Term::lift(built.text(file), span.range(), &names, &ahead);
        let held = self.fields.get_mut(&f).expect("the field");
        held.ty = lift(written.ty.span);
        held.action = written.on_delete.as_ref().map(|a| lift(a.span));
        Ok(file)
    }

    /// Returns field `name` of `ctor` of type declaration `file` as `built` parses it, the
    /// header's when `ctor` is `None`.
    pub(super) fn field_written(
        built: &impl Decls,
        file: FileId,
        ctor: Option<&str>,
        name: &str,
    ) -> Res<ast::FieldType> {
        let Some(ast::Decl::Type(t)) = built.decl(file) else {
            return broken("a type the model printed does not parse");
        };
        let fields: Vec<&ast::FieldType> = match (ctor, &t.body) {
            (None, _) => t.header.iter().flatten().collect(),
            (Some(_), ast::TypeBody::Record(fs)) => fs.iter().collect(),
            (Some(c), ast::TypeBody::Ctors(cs)) => cs
                .iter()
                .filter(|x| x.name.text == c)
                .flat_map(|x| x.record.iter().flatten())
                .collect(),
            _ => Vec::new(),
        };
        match fields.into_iter().find(|f| f.name.text == name) {
            Some(f) => Ok(f.clone()),
            None => broken("a field the model printed does not parse"),
        }
    }

    /// Returns the canonical form of the field `name` of `ctor` of type declaration `file`, the
    /// header's when `ctor` is `None`: through its delete action, or of the action alone.
    pub(super) fn field_canonical(
        built: &Built,
        file: FileId,
        ctor: Option<&str>,
        name: &str,
        action_only: bool,
    ) -> Res<Option<String>> {
        let f = Self::field_written(built, file, ctor, name)?;
        let span = match (&f.on_delete, action_only) {
            (Some(a), true) => a.span,
            (None, true) => return broken("an altered field the model printed has no action"),
            (a, false) => Span {
                start: f.name.span.start,
                end: a.as_ref().map_or(f.ty.span.end, |a| a.span.end),
            },
        };
        Ok(canonical_term(built.text(file), span, &built.names(file)).ok())
    }

    /// Check fill `fill` of field `field_ty` over the rows of constructors `ctors` of `t` in the
    /// declarations now, the whole sum when `None`. Returns its row function and canonical form.
    pub(super) fn fill(
        &self,
        run: &Run,
        t: u64,
        ctors: Option<Vec<u32>>,
        field_ty: &Ty,
        fill: &Fill,
    ) -> Res<(RowFn, String)> {
        let (schema, program) = self.checked(run)?;
        let def = self.def_in(&schema, t).expect("a type the model holds");
        let module = schema
            .module_named(self.module_of((Space::Shared, t)).expect("a declared type"))
            .expect("the type's module");
        let td = schema.ty(def);
        let row = Ty::Data {
            def,
            args: (0..td.params.len() as u32).map(Ty::Param).collect(),
            ctors: ctors.and_then(|cs| schema.normalize_ctors(def, cs)),
        };
        let body = || match fill {
            Fill::Match(arms) => RowBody::Match(arms, run.span()),
            Fill::Expr(e) => RowBody::Expr(e),
        };
        let refs = check::check_row(
            &schema,
            &program,
            module,
            &row,
            body(),
            Some(field_ty),
            RootKind::Fill,
        )
        .or_else(|d| refuse_at(d.span, d.message))?;
        run.note(&schema, &refs);
        let mut names = Names::default();
        for (span, r) in &refs {
            names.note(&schema, *span, r);
        }
        let span = match fill {
            Fill::Expr(e) => e.span,
            Fill::Match(arms) => {
                let first = arms[0].patterns[0].span;
                first.to(arms.last().expect("an arm").body.span)
            }
        };
        let text = canonical_term(run.text(), span, &names)
            .map_err(|u| Fail::Broken(format!("a fill does not resolve: {u:?}")))?;
        let q = check::row_query(
            &schema,
            &program,
            module,
            &row,
            body(),
            Some(field_ty),
            RootKind::Fill,
        )
        .or_else(|d| refuse_at(d.span, d.message))?;
        let query = crate::norm::query(&schema, &program, &q)
            .map_err(|e| Fail::Broken(format!("a fill does not normalise: {e:?}")))?;
        Ok((RowFn { schema, query }, text))
    }

    /// Add field `path.field` of type text `ty` and delete action text `action` to the part
    /// `path` names, every existing row taking `fill`.
    pub(super) fn add_field(
        &mut self,
        run: &Run,
        path: &ast::FieldPath,
        ty: Span,
        action: Option<Span>,
        fill: &Fill,
    ) -> Res<Applied> {
        let (t, ctor) = self.part_of(run, path)?;
        let name = path.field.text.clone();
        if !self.field_free(t, ctor, &name) {
            return refuse_at(
                path.field.span,
                format!(
                    "`{}` already has a field `{name}`",
                    self.shown_part(t, ctor)
                ),
            );
        }
        let id = self.fresh(Space::Field);
        let owner = ctor.unwrap_or(t);
        self.fields.insert(
            id,
            FieldM {
                owner,
                name: name.clone(),
                ty: Term::text(&run.text()[ty.range()]),
                action: action.map(|a| Term::text(&run.text()[a.range()])),
            },
        );
        let held = match ctor {
            Some(c) => &mut self.ctors.get_mut(&c).expect("the constructor").fields,
            None => &mut self.types.get_mut(&t).expect("the type").header,
        };
        held.get_or_insert_with(Vec::new).push(id);
        let built = self.built_for(run)?;
        let file = self.lift_field(&built, id)?;
        let ctor_name = ctor.map(|c| self.ctors[&c].name.clone());
        let decl = Self::field_canonical(&built, file, ctor_name.as_deref(), &name, false)?
            .unwrap_or_else(|| {
                run.late(Late::Field {
                    field: id,
                    action_only: false,
                })
            });
        let after_def = self.def_in(built.schema(), t).expect("the type");
        let ad = built.schema().ty(after_def);
        let fty = match ctor {
            None => ad.header().iter().find(|f| f.name == name),
            Some(c) => {
                let k = self.ctor_index(built.schema(), c).expect("the constructor");
                ad.field_named(k, &name)
            }
        }
        .map(|f| f.ty.clone());
        let Some(fty) = fty else {
            return broken("an added field the schema does not hold");
        };
        let at = ctor.map(|c| {
            vec![
                self.ctor_index(built.schema(), c)
                    .expect("a held constructor"),
            ]
        });
        let (_, fill_text) = self.fill(run, t, at.clone(), &fty, fill)?;
        let mut before = self.clone();
        before.remove_field(id);
        let maps = match before.fill_map(run, t, at, &fty, fill, &name) {
            Ok(map) => Ok(vec![map]),
            Err(Fail::Refused(r)) => Err(r.into_why()),
            Err(e) => return Err(e),
        };
        Ok((
            Resolved::AddField {
                field: id,
                decl,
                fill: fill_text,
            },
            Vec::new(),
            maps,
        ))
    }

    /// Returns the map that gives field `name` of the rows of constructors `at` of `t`, the
    /// whole sum when `None`, their fill, over the declarations now.
    pub(super) fn fill_map(
        &self,
        run: &Run,
        t: u64,
        at: Option<Vec<u32>>,
        fty: &Ty,
        fill: &Fill,
        name: &str,
    ) -> Res<RowMap> {
        let (before, _) = self.checked(run)?;
        let def = self.def_in(&before, t).expect("a type the model holds");
        let all: Vec<u32> = (0..before.ty(def).ctors().len() as u32).collect();
        let (row_fn, _) = self.fill(run, t, at.clone(), fty, fill)?;
        Ok(RowMap::Fill {
            def,
            ctors: at.unwrap_or(all),
            field: name.to_string(),
            fill: row_fn,
        })
    }

    /// Remove field `f` from the part that holds it.
    fn remove_field(&mut self, f: u64) {
        let owner = self.fields[&f].owner;
        let held = match self.types.get_mut(&owner) {
            Some(t) => &mut t.header,
            None => {
                &mut self
                    .ctors
                    .get_mut(&owner)
                    .expect("the field's constructor")
                    .fields
            }
        };
        if let Some(fs) = held {
            fs.retain(|x| *x != f);
        }
        let bare = self.ctors.get(&owner).is_some_and(|c| {
            c.fields.as_ref().is_some_and(Vec::is_empty)
                && matches!(self.types[&c.ty].body, TypeBody::Ctors(_))
        });
        if bare {
            self.ctors.get_mut(&owner).expect("the constructor").fields = None;
        }
        for r in self.relations.values_mut() {
            r.implied.remove(&Implied::Field(f));
            for c in &mut r.clauses {
                match c {
                    ClauseM::Default { ids, .. } => ids.retain(|(x, _)| *x != f),
                    ClauseM::Index { id, elems, .. }
                        if elems
                            .iter()
                            .any(|e| e.objects().any(|o| o == (Space::Field, f))) =>
                    {
                        *id = None;
                    }
                    _ => {}
                }
            }
        }
        self.fields.remove(&f);
    }

    pub(super) fn drop_field(&mut self, run: &Run, path: &ast::FieldPath) -> Res<Applied> {
        let (t, ctor) = self.part_of(run, path)?;
        let Some(f) = self.field_of(t, ctor, &path.field.text) else {
            return refuse_at(
                path.field.span,
                format!(
                    "`{}` has no field `{}`",
                    self.shown_part(t, ctor),
                    path.field.text
                ),
            );
        };
        self.remove_field(f);
        Ok((Resolved::DropField { field: f }, Vec::new(), Ok(Vec::new())))
    }

    /// Give the field `path` names the delete action written at `action`.
    pub(super) fn alter_field(
        &mut self,
        run: &Run,
        path: &ast::FieldPath,
        action: Span,
    ) -> Res<Applied> {
        let (t, ctor) = self.part_of(run, path)?;
        let Some(f) = self.field_of(t, ctor, &path.field.text) else {
            return refuse_at(
                path.field.span,
                format!(
                    "`{}` has no field `{}`",
                    self.shown_part(t, ctor),
                    path.field.text
                ),
            );
        };
        self.fields.get_mut(&f).expect("the field").action =
            Some(Term::text(&run.text()[action.range()]));
        let built = self.built_for(run)?;
        let file = self.lift_field(&built, f)?;
        let ctor_name = ctor.map(|c| self.ctors[&c].name.clone());
        let action =
            Self::field_canonical(&built, file, ctor_name.as_deref(), &path.field.text, true)?
                .unwrap_or_else(|| {
                    run.late(Late::Field {
                        field: f,
                        action_only: true,
                    })
                });
        Ok((
            Resolved::AlterField { field: f, action },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }

    /// Returns the type and constructor a field path names, refusing a newtype and a type whose
    /// kind has no fields.
    pub(super) fn part_of(&self, run: &Run, path: &ast::FieldPath) -> Res<(u64, Option<u64>)> {
        let r = op::Resolving::new(self, run.module());
        let (t, c) = r.part(path)?;
        if let Ok(b) = self.schema_now(run)
            && let Some(def) = self.def_in(b.schema(), t)
            && matches!(b.schema().ty(def).kind, TypeDefKind::Newtype { .. })
        {
            return refuse(format!("newtype `{}` has no fields", self.types[&t].name));
        }
        Ok((t, c))
    }
}
