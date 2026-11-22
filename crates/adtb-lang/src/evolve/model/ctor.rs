use super::field::Applied;
use super::op::Kind;
use super::op::{Res, Resolving, broken, refuse, refuse_at, unmodelled};
use super::run::{Built, Late, Run};
use super::*;
use crate::ast::{self, CtorPlace, CtorSel};
use crate::canon::canonical_term;
use crate::evolve::text::ctor_extent;
use crate::evolve::{ArmTarget, Clause, Cx, GrantSel, Granted, Place, Resolved};
use crate::schema::Stage;
use crate::span::FileId;
use crate::span::Span;

impl Model {
    /// Make record `t` a sum of its one constructor, the key fields of its relation variables
    /// moving to the header.
    fn record_to_sum(&mut self, run: &Run, t: u64) -> Res<()> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
        let Some(def) = schema.ids().type_of(crate::ids::CatTypeId::from_raw(t)) else {
            return broken("a type the built schema does not hold");
        };
        let keys = Cx::key_fields(schema, def);
        let TypeBody::Record { ctor } = self.types[&t].body else {
            return broken("a record that is not one");
        };
        if self.types[&t].header.is_some() {
            return unmodelled("a constructor added to a record with a header");
        }
        let all = self.ctors[&ctor].fields.clone().unwrap_or_default();
        let (head, rest): (Vec<u64>, Vec<u64>) = all
            .iter()
            .partition(|f| keys.contains(&self.fields[f].name));
        let own = if head.is_empty() {
            Some(all)
        } else if rest.is_empty() {
            None
        } else {
            Some(rest)
        };
        for f in &head {
            self.fields.get_mut(f).expect("a key field").owner = t;
        }
        self.ctors
            .get_mut(&ctor)
            .expect("the record's constructor")
            .fields = own;
        let ty = self.types.get_mut(&t).expect("the record");
        ty.header = (!head.is_empty()).then_some(head);
        ty.body = TypeBody::Ctors(vec![ctor]);
        Ok(())
    }

    pub(super) fn add_constructor(
        &mut self,
        run: &Run,
        path: &ast::CtorPath,
        record: Option<&[ast::FieldType]>,
        place: Option<&CtorPlace>,
        joins: &[ast::RelationRef],
        arms: &[ast::ArmClause],
    ) -> Res<Applied> {
        let t = Resolving::new(self, run.module()).ty(&path.ty)?;
        let name = path.ctor.text.clone();
        let tname = self.types[&t].name.clone();
        match self.types[&t].body {
            TypeBody::New { .. } => {
                return refuse_at(
                    path.ty.span(),
                    format!("newtype `{tname}` has no constructors"),
                );
            }
            TypeBody::Record { .. } if name == tname => {
                return refuse_at(
                    path.ctor.span,
                    format!("`{name}` is already a constructor of `{tname}`"),
                );
            }
            TypeBody::Record { .. } => self.record_to_sum(run, t)?,
            TypeBody::Ctors(_) => {}
        }
        self.schema_now(run)?;
        if self.ctor_named(t, &name).is_some() {
            return refuse_at(
                path.ctor.span,
                format!("`{name}` is already a constructor of `{tname}`"),
            );
        }
        let rels = self.relations_of(t);
        for &r in &rels {
            if self.sets_of(r).iter().any(|s| self.sets[s].name == name) {
                return refuse_at(
                    path.ctor.span,
                    format!(
                        "`{name}` is already a named set of `{}`",
                        self.relations[&r].name
                    ),
                );
            }
        }
        let ctors = self.ctors_of(t);
        let (at, placed) = match place {
            None => (ctors.len(), Place::Last),
            Some(CtorPlace::Before(c) | CtorPlace::After(c)) => {
                let Some(k) = ctors.iter().position(|x| self.ctors[x].name == c.text) else {
                    return refuse_at(
                        c.span,
                        format!("`{}` is not a constructor of `{tname}`", c.text),
                    );
                };
                match place {
                    Some(CtorPlace::Before(_)) => (k, Place::Before { ctor: ctors[k] }),
                    _ => (k + 1, Place::After { ctor: ctors[k] }),
                }
            }
        };
        let mut joined = Vec::new();
        {
            let resolving = Resolving::new(self, run.module());
            for j in joins {
                let r = resolving.relation(&j.relation)?;
                if self.relation_type(r) != Some(t) {
                    return refuse_at(
                        j.span,
                        format!(
                            "`{}` is not a relation variable of `{tname}`",
                            self.relations[&r].name
                        ),
                    );
                }
                let Some(CtorSel::One(set)) = &j.sel else {
                    return refuse_at(j.span, "`joins` names a named set: `r.S`");
                };
                joined.push(resolving.set(set, &j.relation)?);
            }
        }
        let c = self.fresh(Space::Shared);
        let fields = record.map(|fs| {
            fs.iter()
                .map(|f| {
                    let id = self.fresh(Space::Field);
                    self.fields.insert(
                        id,
                        FieldM {
                            owner: c,
                            name: f.name.text.clone(),
                            ty: Term::text(&run.text()[f.ty.span.range()]),
                            action: f
                                .on_delete
                                .as_ref()
                                .map(|a| Term::text(&run.text()[a.span.range()])),
                        },
                    );
                    id
                })
                .collect::<Vec<u64>>()
        });
        self.ctors.insert(
            c,
            CtorM {
                ty: t,
                name: name.clone(),
                fields,
            },
        );
        if let TypeBody::Ctors(cs) = &mut self.types.get_mut(&t).expect("the type").body {
            cs.insert(at, c);
        }
        for &s in &joined {
            self.sets
                .get_mut(&s)
                .expect("a joined set")
                .members
                .push(Term::name((Space::Shared, c), &name));
        }
        let declared = State {
            stage: Stage::Declared,
            since: self.version,
        };
        for &r in &rels {
            self.states.insert((r, c), declared);
        }
        self.value_states.insert(c, declared);
        let built = match self.build_adopting(&[], &run.ahead(self)) {
            Ok(b) => b,
            Err(ds) if !arms.is_empty() => return refuse(ds[0].diag.message.clone()),
            Err(_) => return unmodelled("a constructor added to declarations that do not build"),
        };
        let file = self.relift(&built, Item::Type(t))?;
        self.widen_grants(t, c, &rels);
        let decl = self.ctor_canonical(&built, run, file, c, record.is_some())?;
        let mut clauses: Vec<Clause> = joined
            .into_iter()
            .map(|set| Clause::Joins { set })
            .collect();
        if !arms.is_empty() {
            let (edits, written) = self.arm_writes(run, &built, t, (&[c], &[]), arms)?;
            self.edit_decls(edits)?;
            let after = self.build_adopting(&[], &run.ahead(self)).ok();
            clauses.extend(self.arm_clauses(run, after.as_ref(), &[c], written)?);
        }
        Ok((
            Resolved::AddConstructor {
                ctor: c,
                sum: t,
                decl,
                place: placed,
            },
            clauses,
            Ok(Vec::new()),
        ))
    }

    /// Returns the canonical form of constructor `c` of the type declared in `file` of `built`,
    /// from its name through its record, empty with a note when it names what only the
    /// migration's end resolves.
    pub(super) fn ctor_canonical(
        &self,
        built: &Built,
        run: &Run,
        file: FileId,
        c: u64,
        record: bool,
    ) -> Res<String> {
        Ok(self
            .ctor_written(built, file, c, record)?
            .unwrap_or_else(|| run.late(Late::Ctor { ctor: c, record })))
    }

    /// Returns the canonical form of constructor `c` of the type declared in `file` of `built`,
    /// from its name through its record, `None` when a name of it does not resolve there.
    pub(super) fn ctor_written(
        &self,
        built: &Built,
        file: FileId,
        c: u64,
        record: bool,
    ) -> Res<Option<String>> {
        self.ctors_written(built, file, (c, c), record)
    }

    /// Returns the canonical form of the constructors of the type declared in `file` of `built`
    /// from the name of `first` through `last` and its record, `None` when a name of them does
    /// not resolve there.
    pub(super) fn ctors_written(
        &self,
        built: &Built,
        file: FileId,
        (first, last): (u64, u64),
        record: bool,
    ) -> Res<Option<String>> {
        let Some(ast::Decl::Type(td)) = built.decl(file) else {
            return broken("a type the model printed does not parse");
        };
        let ast::TypeBody::Ctors(cs) = &td.body else {
            return broken("a sum the model printed as no sum");
        };
        let find = |c: u64| cs.iter().find(|x| x.name.text == self.ctors[&c].name);
        let (Some(a), Some(z)) = (find(first), find(last)) else {
            return broken("a constructor the model printed is missing");
        };
        let end = ctor_extent(built.text(file), &z.name, record);
        Ok(canonical_term(
            built.text(file),
            Span::new(a.name.span.start as usize, end.end),
            &built.names(file),
        )
        .ok())
    }

    /// Give each `read` and `update` grant open to new constructor `c` of type `t`, whose relation
    /// variables are `rels`, the fields of `c`: a grant on a whole relation variable without a
    /// field list, and one on a named set `c` joins, keeping only the names a list states.
    fn widen_grants(&mut self, t: u64, c: u64, rels: &[u64]) {
        let joined: BTreeSet<u64> = self
            .sets
            .iter()
            .filter(|(_, s)| {
                self.relation_type(s.relation) == Some(t)
                    && s.members
                        .iter()
                        .any(|m| m.objects().any(|o| o == (Space::Shared, c)))
            })
            .map(|(&id, _)| id)
            .collect();
        let own: Vec<(String, u64)> = self.ctors[&c]
            .fields
            .iter()
            .flatten()
            .map(|f| (self.fields[f].name.clone(), *f))
            .collect();
        let fields = &self.fields;
        for g in &mut self.grants {
            let (Granted::Read(target, fs) | Granted::Update(target, fs)) = &mut g.privilege else {
                continue;
            };
            let open = match target.sel {
                GrantSel::Whole => rels.contains(&target.relation),
                GrantSel::Set(s) => joined.contains(&s),
                GrantSel::Ctors(_) => false,
            };
            if !open || (fs.listed && target.sel == GrantSel::Whole) {
                continue;
            }
            let listed: BTreeSet<&str> = fs
                .ids
                .iter()
                .filter_map(|f| fields.get(f).map(|x| x.name.as_str()))
                .collect();
            let gained: Vec<u64> = own
                .iter()
                .filter(|(n, _)| !fs.listed || listed.contains(n.as_str()))
                .map(|&(_, id)| id)
                .collect();
            fs.ids.extend(gained);
            fs.ids.sort_unstable();
            fs.ids.dedup();
        }
    }

    /// Returns the declaration an `arm` clause names, the target it resolves to, and for a
    /// computed field the position of its clause.
    pub(super) fn arm_target(
        &self,
        run: &Run,
        a: &ast::ArmClause,
    ) -> Res<(Item, ArmTarget, Option<usize>)> {
        let resolving = Resolving::new(self, run.module());
        let m = resolving.module_of(&a.target);
        if let Some(f) = &a.field {
            let r = resolving.relation(&a.target)?;
            let found = self.relations[&r]
                .clauses
                .iter()
                .enumerate()
                .find_map(|(k, c)| match c {
                    ClauseM::Computed { id, name, .. } if *name == f.text => Some((k, *id)),
                    _ => None,
                });
            let Some((k, Some(computed))) = found else {
                return refuse_at(
                    f.span,
                    format!(
                        "`{}` has no computed field `{}`",
                        a.target.name.text, f.text
                    ),
                );
            };
            return Ok((
                Item::Relation(r),
                ArmTarget::OfComputed { computed },
                Some(k),
            ));
        }
        match self.find(&m, Kind::Fn, &a.target.name.text) {
            Some(Item::Fn(function)) => {
                Ok((Item::Fn(function), ArmTarget::OfFn { function }, None))
            }
            _ => match self.find(&m, Kind::Constraint, &a.target.name.text) {
                Some(Item::Constraint(constraint)) => Ok((
                    Item::Constraint(constraint),
                    ArmTarget::OfConstraint { constraint },
                    None,
                )),
                _ => refuse_at(
                    a.target.span(),
                    format!(
                        "module `{m}` declares no schema function, constraint or computed field `{}`",
                        a.target.name.text
                    ),
                ),
            },
        }
    }
}
