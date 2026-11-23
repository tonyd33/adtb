use super::field::Applied;
use super::op::{Fail, Res, Resolving, broken, refuse, refuse_at};
use super::revert::{How, Undone};
use super::run::Run;
use super::*;
use crate::ast::{self, Fill};
use crate::check::{self, Ref};
use crate::evolve::revert::{DeclOf, Late as Gone, NamedId, PlacedArms, Spelled, Undo, dropped};
use crate::evolve::{Resolved, RowMap, holds, match_arms};
use crate::ids::{CatCtorId, CatSetId, KeyKind};
use crate::parse::{Options, parse_schema};
use crate::span::FileId;

/// The parts of a structural inverse: the uniques and indexes it drops first, its operator, the
/// operators that add their replacements, how it applies its operator, and the text each check
/// takes.
pub(super) type Restructuring = (
    Vec<crate::ids::CatIndexId>,
    Spelled,
    Spelled,
    Undo,
    Vec<(crate::ids::CatConstraintId, Spelled)>,
);

impl Model {
    /// Apply a structural inverse: drop its uniques and indexes, apply its operator as `how`
    /// says, setting its checks within that step, then rename an unnest's constructors back and
    /// apply the operators that add the replacements.
    pub(super) fn restructure(
        &mut self,
        run: &mut Run,
        (drops, op, adds, how, checks): Restructuring,
        of: &str,
        gone: &Gone,
        undone: &mut Undone,
    ) -> Res<()> {
        let text = self.spell_copies(&op, of, gone)?;
        let adds = self.spell_copies(&adds, of, gone)?;
        let mut set = Vec::new();
        for (c, t) in checks {
            let Some(item) = self.decl_of(DeclOf::Object(KeyKind::Constraint, c.raw())) else {
                return refuse(dropped(of, "constraint"));
            };
            set.push((item, self.spell(&t, of, gone)?));
        }
        let mut from = Vec::new();
        let mut renames = None;
        match &how {
            Undo::UpFrom(ctors) => {
                for c in ctors {
                    if !self.ctors.contains_key(&c.raw()) {
                        return refuse(dropped(of, "constructor"));
                    }
                    from.push(c.raw());
                }
            }
            Undo::Unnest(ty, products) => {
                let schema = self.schema_now(run)?.into_schema();
                let Some((module, t)) = NamedId::Type(*ty).spelled(&schema) else {
                    return refuse(dropped(of, "type"));
                };
                let mut moved = Vec::new();
                for (c, was) in products {
                    let Some(n) = self.ctors.get(&c.raw()).map(|x| x.name.clone()) else {
                        return refuse(dropped(of, "constructor"));
                    };
                    if n != *was {
                        moved.push((n, was.clone()));
                    }
                }
                let now: BTreeSet<String> = self
                    .named_keys()
                    .keys()
                    .filter_map(|k| k.name().map(str::to_string))
                    .collect();
                let taken: BTreeSet<&str> = now
                    .iter()
                    .map(String::as_str)
                    .chain(moved.iter().map(|(_, w)| w.as_str()))
                    .collect();
                let mut through: Vec<String> = Vec::new();
                for (n, _) in &moved {
                    let fresh = (1..)
                        .map(|k| format!("{n}{k}"))
                        .find(|x| !taken.contains(x.as_str()) && !through.contains(x))
                        .expect("an unused name");
                    through.push(fresh);
                }
                let mut text = String::new();
                for ((n, _), x) in moved.iter().zip(&through) {
                    text.push_str(&format!("rename constructor {t}.{n} to {x}\n"));
                }
                for ((_, was), x) in moved.iter().zip(&through) {
                    text.push_str(&format!("rename constructor {t}.{x} to {was}\n"));
                }
                renames = (!text.is_empty()).then_some((module, text));
            }
            Undo::Written | Undo::Derived => {}
        }
        if !drops.is_empty() {
            for id in drops {
                if !self.drop_index_id(id.raw()) {
                    return refuse(dropped(of, "index"));
                }
            }
            self.settle_ids(run);
            undone.unmapped(self);
        }
        let how = match how {
            Undo::Written | Undo::Unnest(..) => How::Written,
            Undo::Derived => How::Derived,
            Undo::UpFrom(_) => How::UpFrom(&from),
        };
        if !text.text.is_empty() {
            self.undo_ops(run, run.module(), text, how, &set, undone)?;
        } else if !set.is_empty() {
            for (item, check) in &set {
                self.redeclare(*item, check)?;
            }
            self.settle_ids(run);
            undone.unmapped(self);
        }
        if let Some((module, text)) = renames {
            self.undo_ops(run, &module, text.into(), How::Written, &[], undone)?;
        }
        if !adds.text.is_empty() {
            self.undo_ops(run, run.module(), adds, How::Written, &[], undone)?;
        }
        Ok(())
    }

    /// Drop the unique or index of id `id`. Returns false when no relation variable holds it.
    fn drop_index_id(&mut self, id: u64) -> bool {
        for r in self.relations.values_mut() {
            if let Some(k) = r
                .clauses
                .iter()
                .position(|c| matches!(c, ClauseM::Index { id: Some(x), .. } if *x == id))
            {
                r.clauses.remove(k);
                return true;
            }
        }
        false
    }

    /// Remove constructor `ctor` with its arms as the inverse of adding it, then, with `record`,
    /// set its type, when a sum of one constructor named after it, to a record of its fields,
    /// those of `record` first in that order.
    pub(super) fn unadd(
        &mut self,
        run: &mut Run,
        ctor: u64,
        record: Option<Vec<crate::ids::CatFieldId>>,
        of: &str,
    ) -> Res<()> {
        let Some(t) = self.ctors.get(&ctor).map(|c| c.ty) else {
            return refuse(dropped(of, "constructor"));
        };
        self.remove_ctor(run, t, ctor, true)?;
        let Some(order) = record else {
            return Ok(());
        };
        let tname = self.types[&t].name.clone();
        let one = match &self.types[&t].body {
            TypeBody::Ctors(cs) if cs.len() == 1 && self.ctors[&cs[0]].name == tname => cs[0],
            _ => return Ok(()),
        };
        let header = self
            .types
            .get_mut(&t)
            .expect("the type")
            .header
            .take()
            .unwrap_or_default();
        for f in &header {
            self.fields.get_mut(f).expect("a header field").owner = one;
        }
        let own = self.ctors[&one].fields.clone().unwrap_or_default();
        let mut all: Vec<u64> = header.into_iter().chain(own).collect();
        let order: Vec<u64> = order.iter().map(|f| f.raw()).collect();
        all.sort_by_key(|f| order.iter().position(|x| x == f).unwrap_or(usize::MAX));
        self.ctors.get_mut(&one).expect("the constructor").fields = Some(all);
        self.types.get_mut(&t).expect("the type").body = TypeBody::Record { ctor: one };
        if !self.unlifted.contains(&Item::Type(t)) {
            self.unlifted.push(Item::Type(t));
        }
        self.settle_ids(run);
        Ok(())
    }

    /// Put back constructor `name` of type `ty` as `text`, which held id `ctor`, after the
    /// constructor `after` names, else first; as a member of each set of `joins`; and with its
    /// arms in each constraint of `arms`.
    pub(super) fn readd(
        &mut self,
        run: &mut Run,
        (ty, ctor, name): (u64, u64, &str),
        text: &str,
        (after, joins, arms): (Option<CatCtorId>, Vec<CatSetId>, Vec<PlacedArms>),
        of: &str,
        gone: &Gone,
    ) -> Res<()> {
        if !self.types.contains_key(&ty) {
            return refuse(dropped(of, "type"));
        }
        let after = match after {
            Some(c) if self.ctors.contains_key(&c.raw()) => Some(c.raw()),
            Some(_) => return refuse(dropped(of, "constructor")),
            None => None,
        };
        let mut sets = Vec::new();
        for s in joins {
            if !self.sets.contains_key(&s.raw()) {
                return refuse(dropped(of, "set"));
            }
            sets.push(s.raw());
        }
        let built = self.schema_now(run)?;
        let schema = built.schema();
        let def = schema
            .ids()
            .type_of(crate::ids::CatTypeId::from_raw(ty))
            .expect("a held type");
        let refs = check::schema_refs(schema);
        let mut placed = Vec::new();
        for (c, n, beside, texts) in arms {
            let Some(item) = self.decl_of(DeclOf::Object(KeyKind::Constraint, c.raw())) else {
                return refuse(dropped(of, "constraint"));
            };
            let beside = match beside {
                Some(b) => match schema.ids().ctor_of(b) {
                    Some(r) if r.def() == def => Some(r.index()),
                    _ => return refuse(dropped(of, "constructor")),
                },
                None => None,
            };
            let k = self.decl_at(item).expect("a held constraint");
            let file = FileId(k as u32);
            let at = refs
                .iter()
                .filter(|(f, _, r)| {
                    *f == file && matches!(r, Ref::Match(x) if over(schema, x, def))
                })
                .nth(n);
            let (Some((_, at, _)), Some(d)) = (at, built.decl(file)) else {
                continue;
            };
            let arms = match_arms(d, *at).unwrap_or_default();
            let place = match beside {
                Some(b) => arms
                    .iter()
                    .find(|a| arm_ctor(&refs, file, def, a) == Some(b))
                    .or(arms.last())
                    .map(|a| (a.body.span.end as usize, false)),
                None => arms
                    .first()
                    .map(|a| (a.patterns[0].span.start as usize, true)),
            };
            if let Some((end, first)) = place {
                let header = format!("module {}\n", self.decls[k].module).len();
                placed.push((item, end - header, first, texts));
            }
        }
        let c = self.fresh(Space::Shared);
        let (cname, fields) = ctor_parts(text).ok_or(Fail::Broken(
            "a constructor put back whose text does not parse".into(),
        ))?;
        let mut held = None;
        if let Some(fs) = fields {
            let mut ids = Vec::new();
            for (fname, fty, action) in fs {
                let id = self.fresh(Space::Field);
                self.fields.insert(
                    id,
                    FieldM {
                        owner: c,
                        name: fname,
                        ty: Term::text(&fty),
                        action: action.map(|a| Term::text(&a)),
                    },
                );
                ids.push(id);
            }
            held = Some(ids);
        }
        self.ctors.insert(
            c,
            CtorM {
                ty,
                name: cname,
                fields: held,
            },
        );
        let body = &mut self.types.get_mut(&ty).expect("the type").body;
        match body {
            TypeBody::Ctors(cs) => {
                let at = after
                    .and_then(|a| cs.iter().position(|x| *x == a))
                    .map_or(0, |k| k + 1);
                cs.insert(at, c);
            }
            TypeBody::Record { ctor: one } => {
                let one = *one;
                *body = if after.is_some() {
                    TypeBody::Ctors(vec![one, c])
                } else {
                    TypeBody::Ctors(vec![c, one])
                };
            }
            TypeBody::New { .. } => return broken("a constructor put back into a newtype"),
        }
        if !self.unlifted.contains(&Item::Type(ty)) {
            self.unlifted.push(Item::Type(ty));
        }
        for s in sets {
            self.sets
                .get_mut(&s)
                .expect("a held set")
                .members
                .push(Term::name((Space::Shared, c), name));
        }
        let again: crate::evolve::revert::Recreated =
            [((Space::Shared, ctor), c)].into_iter().collect();
        placed.sort_by_key(|p| std::cmp::Reverse(p.1));
        for (item, at, first, texts) in placed {
            let mut joined = String::new();
            for t in texts {
                let t = self.spell(&t.mapped(&again), of, gone)?;
                if first {
                    joined.push_str(&t);
                    joined.push_str("\n  | ");
                } else {
                    joined.push_str("\n  | ");
                    joined.push_str(&t);
                }
            }
            let i = self.decl_at(item).expect("a held constraint");
            let printed = self.print(i);
            let model = self.clone();
            let Some(rest) = self.body_term(item) else {
                return broken("arms put back into a declaration with no body");
            };
            let prefix = printed.len() - rest.print(&model).len();
            rest.splice(&model, at - prefix, &joined)
                .map_err(Fail::Broken)?;
            if !self.unlifted.contains(&item) {
                self.unlifted.push(item);
            }
        }
        Ok(())
    }

    /// Returns the term after the name of the function, aggregate, constraint or check `item`.
    fn body_term(&mut self, item: Item) -> Option<&mut Term> {
        match item {
            Item::Fn(f) | Item::Aggregate(f) => self.functions.get_mut(&f).map(|b| &mut b.rest),
            Item::Constraint(c) | Item::Check(c) => self.denials.get_mut(&c).map(|b| &mut b.rest),
            _ => None,
        }
    }

    /// Set relation variable `r`'s key clause to `text`, its generator included.
    pub(super) fn key_clause(&mut self, r: u64, text: &str) -> Res<()> {
        let src = format!("module m\nrelation k : K\n  {text}");
        let Some(ast::Decl::Relation(rd)) = parse_schema(&src, Options { elisions: true })
            .ok()
            .ok()
            .and_then(|f| f.decls.into_iter().next())
        else {
            return broken("a key clause set back that does not parse");
        };
        let Some(ast::RelationClause::Key(fields, generate)) = rd.clauses.first() else {
            return broken("a key clause set back that is no key");
        };
        let rel = self
            .relations
            .get_mut(&r)
            .expect("a held relation variable");
        let Some(ClauseM::Key {
            fields: held,
            listed,
            generate: g,
            ..
        }) = rel
            .clauses
            .iter_mut()
            .find(|c| matches!(c, ClauseM::Key { .. }))
        else {
            return Ok(());
        };
        *held = fields.iter().map(|f| Term::text(&f.text)).collect();
        *listed = src[..fields[0].span.start as usize]
            .trim_end()
            .ends_with('(');
        *g = generate.as_ref().map(|x| {
            let id = g.as_ref().and_then(|(_, id)| *id);
            (x.text.clone(), id)
        });
        if !self.unlifted.contains(&Item::Relation(r)) {
            self.unlifted.push(Item::Relation(r));
        }
        Ok(())
    }

    /// Move field `path` names of each constructor of `from` to the header, the other
    /// constructors taking `fill`, the header field keeping the id of the field of `path`'s
    /// constructor.
    pub(super) fn move_up_from(
        &mut self,
        run: &mut Run,
        path: &ast::FieldPath,
        from: &[u64],
        fill: Option<&Fill>,
    ) -> Res<Applied> {
        let t = Resolving::new(self, run.module()).ty(&path.ty)?;
        let tname = self.types[&t].name.clone();
        if !matches!(self.types[&t].body, TypeBody::Ctors(_)) {
            return refuse_at(
                path.ty.span(),
                format!("record `{tname}` has no header to move a field to or from"),
            );
        }
        let name = path.field.text.clone();
        let mut moved = Vec::new();
        for &c in from {
            let Some(f) = self.field_of(t, Some(c), &name) else {
                return refuse(format!(
                    "`{tname}.{}` has no field `{name}`",
                    self.ctors[&c].name
                ));
            };
            moved.push((c, f));
        }
        let others: Vec<u64> = self
            .ctors_of(t)
            .into_iter()
            .filter(|o| !from.contains(o))
            .collect();
        if let Some(&o) = others
            .iter()
            .find(|&&o| self.field_of(t, Some(o), &name).is_some())
        {
            return refuse(format!(
                "`{tname}.{}` already has a field `{name}`; the header cannot take it",
                self.ctors[&o].name
            ));
        }
        let holder = path
            .ctor
            .as_ref()
            .and_then(|c| self.ctor_named(t, &c.text))
            .and_then(|c| moved.iter().find(|(x, _)| *x == c))
            .or(moved.first())
            .map(|(_, f)| *f)
            .ok_or(Fail::Broken("a move up from no constructor".into()))?;
        let before = self.clone();
        let mut merged = BTreeMap::new();
        for &(c, f) in &moved {
            self.unplace_field(f);
            if self.ctors[&c].fields.as_ref().is_some_and(Vec::is_empty) {
                self.ctors.get_mut(&c).expect("the constructor").fields = None;
            }
            if f != holder {
                self.fields.remove(&f);
                merged.insert((Space::Field, f), (Space::Field, holder));
            }
        }
        self.terms_mut(&mut |x| x.remap(&merged));
        self.place_field(t, None, holder);
        let fill_text = if others.is_empty() {
            None
        } else {
            let Some(fill) = fill else {
                return refuse(
                    "a move to the header takes `fill match` with an arm for each constructor that lacks the field",
                );
            };
            if matches!(fill, Fill::Expr(_)) {
                return refuse(
                    "the fill of a move to the header is a `match` whose arms are the constructors that lack the field",
                );
            }
            let built = self.built_for(run)?;
            let def = self.def_in(built.schema(), t).expect("the type");
            let ks: Vec<u32> = others
                .iter()
                .filter_map(|&o| self.ctor_index(built.schema(), o))
                .collect();
            let Some(fty) = built
                .schema()
                .ty(def)
                .header()
                .iter()
                .find(|x| x.name == name)
                .map(|x| x.ty.clone())
            else {
                return refuse("the moved field does not resolve");
            };
            Some(self.fill(run, t, Some(ks), &fty, fill)?.1)
        };
        self.built_for(run)?;
        let ctors: Vec<u64> = moved.iter().map(|(c, _)| *c).collect();
        let maps = before.up_maps(run, t, &ctors, &others, &name, fill);
        let maps = maps.map(|ms: Vec<RowMap>| ms);
        Ok((
            Resolved::MoveField {
                field: holder,
                header: true,
                fill: fill_text,
            },
            Vec::new(),
            maps,
        ))
    }
}

/// Returns whether type `ty` of `schema` is type `def` or a constructor set of it.
fn over(schema: &crate::schema::Schema, ty: &crate::types::Ty, def: crate::types::TypeId) -> bool {
    (0..schema.ty(def).ctors().len() as u32).any(|c| holds(schema, ty, def, c))
}

/// Returns the constructor of type `def` the first pattern of arm `a` of file `file` names.
fn arm_ctor(
    refs: &[(FileId, crate::span::Span, Ref)],
    file: FileId,
    def: crate::types::TypeId,
    a: &ast::Arm,
) -> Option<u32> {
    let ast::PatternKind::Ctor { path, ctor, .. } = &a.patterns.first()?.kind else {
        return None;
    };
    let name = ctor.as_ref().unwrap_or(&path.name).span;
    refs.iter().find_map(|(f, s, r)| match r {
        Ref::Ctor(d, c) if *f == file && *s == name && *d == def => Some(*c),
        _ => None,
    })
}

/// A field of a constructor's text: its name, type and delete action as written.
type FieldText = (String, String, Option<String>);

/// Returns the name of constructor text `text`, `C` or `C { f: T, ... }`, and its fields.
fn ctor_parts(text: &str) -> Option<(String, Option<Vec<FieldText>>)> {
    let src = format!("module m\ntype X = {text} | Y");
    let file = parse_schema(&src, Options { elisions: true }).ok().ok()?;
    let ast::Decl::Type(t) = file.decls.first()? else {
        return None;
    };
    let ast::TypeBody::Ctors(cs) = &t.body else {
        return None;
    };
    let c = cs.first()?;
    let fields = c.record.as_ref().map(|fs| {
        fs.iter()
            .map(|f| {
                (
                    f.name.text.clone(),
                    src[f.ty.span.range()].to_string(),
                    f.on_delete
                        .as_ref()
                        .map(|a| src[a.span.range()].to_string()),
                )
            })
            .collect()
    });
    Some((c.name.text.clone(), fields))
}
