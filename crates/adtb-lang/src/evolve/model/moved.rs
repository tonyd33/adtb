use super::field::{Applied, Maps};
use super::op::{Fail, Res, Resolving, broken, refuse, refuse_at};
use super::run::{Built, Late, Run};
use super::*;
use crate::ast::{self, CtorSel, Fill, QName};
use crate::canon::{Names, canonical_term};
use crate::evolve::ctor::fill_image;
use crate::evolve::field::{carried_fields, path_text};
use crate::evolve::{Clause, Granted, Resolved, RowMap};
use crate::ids::CatRelId;
use crate::schema::Schema;

fn sel_names(sel: &CtorSel) -> Vec<&ast::Ident> {
    match sel {
        CtorSel::One(c) => vec![c],
        CtorSel::Many(cs) => cs.iter().collect(),
    }
}

impl Model {
    pub(super) fn move_field(
        &mut self,
        run: &Run,
        path: &ast::FieldPath,
        to: &QName,
        sel: Option<&CtorSel>,
        fill: Option<&Fill>,
    ) -> Res<Applied> {
        let resolving = Resolving::new(self, run.module());
        resolving.ty(&path.ty)?;
        if to.name.text != path.ty.name.text
            || resolving.module_of(to) != resolving.module_of(&path.ty)
        {
            return refuse_at(to.span(), "a field moves within its own type");
        }
        let (t, ctor) = self.part_of(run, path)?;
        let name = path.field.text.clone();
        let tname = self.types[&t].name.clone();
        let Some(f) = self.field_of(t, ctor, &name) else {
            return refuse_at(
                path.field.span,
                format!("`{}` has no field `{name}`", self.shown_part(t, ctor)),
            );
        };
        let TypeBody::Ctors(ctors) = self.types[&t].body.clone() else {
            return refuse_at(
                path.ty.span(),
                format!("record `{tname}` has no header to move a field to or from"),
            );
        };
        match (ctor, sel) {
            (Some(c), None) => self.move_up(run, t, c, f, fill),
            (None, Some(sel)) => {
                if fill.is_some() {
                    return refuse("a move from the header down to constructors takes no fill");
                }
                let mut targets = Vec::new();
                for x in sel_names(sel) {
                    let Some(c) = ctors.iter().copied().find(|c| self.ctors[c].name == x.text)
                    else {
                        return refuse_at(
                            x.span,
                            format!("`{}` is not a constructor of `{tname}`", x.text),
                        );
                    };
                    targets.push(c);
                }
                self.move_down(run, t, f, &targets)
            }
            (Some(_), Some(_)) => refuse(
                "a constructor's field moves to the header: `move field T.C.f to T fill match ...`",
            ),
            (None, None) => {
                refuse("a header field moves down to constructors: `move field T.f to T.(C | D)`")
            }
        }
    }

    /// Put field `f` of `t` into the record of `c`, at its end.
    pub(super) fn place_field(&mut self, t: u64, c: Option<u64>, f: u64) {
        self.fields.get_mut(&f).expect("the field").owner = c.unwrap_or(t);
        let held = match c {
            Some(c) => &mut self.ctors.get_mut(&c).expect("the constructor").fields,
            None => &mut self.types.get_mut(&t).expect("the type").header,
        };
        held.get_or_insert_with(Vec::new).push(f);
    }

    /// Take field `f` out of the part of `t` that holds it.
    pub(super) fn unplace_field(&mut self, f: u64) {
        let owner = self.fields[&f].owner;
        let held = match self.types.get_mut(&owner) {
            Some(t) => &mut t.header,
            None => &mut self.ctors.get_mut(&owner).expect("the constructor").fields,
        };
        if let Some(fs) = held {
            fs.retain(|x| *x != f);
        }
    }

    /// Move header field `f` of `t` down to each of `targets`, the first by declaration order
    /// keeping its id.
    fn move_down(&mut self, run: &Run, t: u64, f: u64, targets: &[u64]) -> Res<Applied> {
        let before = self.clone();
        let schema = self.schema_now(run)?.into_schema();
        let def = self.def_in(&schema, t).expect("the type");
        let order = self.ctors_of(t);
        let mut placed: Vec<u64> = targets.to_vec();
        placed.sort_by_key(|c| order.iter().position(|x| x == c));
        placed.dedup();
        let held = self.fields[&f].clone();
        self.unplace_field(f);
        let mut given = BTreeMap::new();
        for (k, &c) in placed.iter().enumerate() {
            let id = if k == 0 {
                f
            } else {
                let id = self.fresh(Space::Field);
                self.fields.insert(id, held.clone());
                id
            };
            self.place_field(t, Some(c), id);
            given.insert(c, id);
        }
        let built = self.built_for(run)?;
        let after = built.schema();
        for g in &mut self.grants {
            if let Granted::Read(target, fs) | Granted::Update(target, fs) = &mut g.privilege
                && fs.ids.contains(&f)
            {
                let covered = target.ctors(after);
                fs.ids.retain(|&x| x != f);
                fs.ids.extend(
                    given
                        .iter()
                        .filter(|(c, _)| covered.contains(c))
                        .map(|(_, x)| *x),
                );
                fs.ids.sort_unstable();
                fs.ids.dedup();
            }
        }
        let ks: Vec<u32> = targets
            .iter()
            .filter_map(|&c| before.ctor_index(&schema, c))
            .collect();
        let maps = Ok(vec![RowMap::Copy {
            def,
            ctors: ks,
            field: held.name,
        }]);
        Ok((
            Resolved::MoveField {
                field: given[&targets[0]],
                header: false,
                fill: None,
            },
            targets
                .iter()
                .map(|&ctor| Clause::Member { ctor })
                .collect(),
            maps,
        ))
    }

    /// Move field `f` of constructor `c` of `t` to the header, the other constructors taking
    /// `fill`.
    fn move_up(&mut self, run: &Run, t: u64, c: u64, f: u64, fill: Option<&Fill>) -> Res<Applied> {
        let before = self.clone();
        let name = self.fields[&f].name.clone();
        let tname = self.types[&t].name.clone();
        let others: Vec<u64> = self.ctors_of(t).into_iter().filter(|&o| o != c).collect();
        if let Some(&o) = others
            .iter()
            .find(|&&o| self.field_of(t, Some(o), &name).is_some())
        {
            return refuse(format!(
                "`{tname}.{}` already has a field `{name}`; the header cannot take it",
                self.ctors[&o].name
            ));
        }
        self.unplace_field(f);
        if self.ctors[&c].fields.as_ref().is_some_and(Vec::is_empty) {
            self.ctors.get_mut(&c).expect("the constructor").fields = None;
        }
        self.place_field(t, None, f);
        if others.is_empty() {
            self.built_for(run)?;
            let maps = before.up_maps(run, t, &[c], &[], &name, None);
            return Ok((
                Resolved::MoveField {
                    field: f,
                    header: true,
                    fill: None,
                },
                Vec::new(),
                maps,
            ));
        }
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
        let (_, text) = self.fill(run, t, Some(ks), &fty, fill)?;
        self.lift_grants(built.schema(), t, c, f, fill);
        let maps = before.up_maps(run, t, &[c], &others, &name, Some(fill));
        Ok((
            Resolved::MoveField {
                field: f,
                header: true,
                fill: Some(text),
            },
            Vec::new(),
            maps,
        ))
    }

    /// Returns the maps of a move of field `name` of each constructor of `from` of `t` to the
    /// header over the declarations now, the constructors `others` taking `fill`.
    pub(super) fn up_maps(
        &self,
        run: &Run,
        t: u64,
        from: &[u64],
        others: &[u64],
        name: &str,
        fill: Option<&Fill>,
    ) -> Maps {
        let refused = Fail::reason;
        let built = self.schema_now(run).map_err(refused)?;
        let schema = built.schema();
        let def = self.def_in(schema, t).expect("the type");
        let ks: Vec<u32> = from
            .iter()
            .map(|&c| self.ctor_index(schema, c).expect("the constructor"))
            .collect();
        let Some(fty) = schema
            .ty(def)
            .field_named(ks[0], name)
            .map(|x| x.ty.clone())
        else {
            let names: Vec<&str> = from.iter().map(|c| self.ctors[c].name.as_str()).collect();
            return Err(format!("`{}` has no field `{name}`", names.join(" | ")));
        };
        let mut maps = vec![RowMap::Copy {
            def,
            ctors: ks,
            field: name.to_string(),
        }];
        if let Some(fill) = fill
            && !others.is_empty()
        {
            let at: Vec<u32> = others
                .iter()
                .filter_map(|&o| self.ctor_index(schema, o))
                .collect();
            maps.push(
                self.fill_map(run, t, Some(at), &fty, fill, name)
                    .map_err(refused)?,
            );
        }
        Ok(maps)
    }

    /// Take header field `f`, moved up from constructor `c` of `t`, from each grant covering a
    /// constructor other than `c` whose value `fill` does not carry whole from a field the
    /// grant covers.
    fn lift_grants(&mut self, schema: &Schema, t: u64, c: u64, f: u64, fill: &Fill) {
        let arms: Vec<(Option<String>, Option<String>)> = match fill {
            Fill::Match(arms) => arms.iter().map(fill_image).collect(),
            Fill::Expr(_) => vec![(None, None)],
        };
        let field_id = |d: u64, g: &str| self.field_of(t, Some(d), g);
        let mut lost = Vec::new();
        for (k, g) in self.grants.iter().enumerate() {
            let (Granted::Read(target, fs) | Granted::Update(target, fs)) = &g.privilege else {
                continue;
            };
            if !fs.ids.contains(&f) {
                continue;
            }
            let carried = target
                .ctors(schema)
                .into_iter()
                .filter(|&d| d != c && self.ctors.contains_key(&d))
                .all(|d| {
                    let dn = &self.ctors[&d].name;
                    let reaching: Vec<&Option<String>> = arms
                        .iter()
                        .filter(|(from, _)| from.as_ref().is_none_or(|x| x == dn))
                        .map(|(_, f)| f)
                        .collect();
                    !reaching.is_empty()
                        && reaching.iter().all(|g| {
                            g.as_ref()
                                .and_then(|g| field_id(d, g))
                                .is_some_and(|x| fs.ids.contains(&x))
                        })
                });
            if !carried {
                lost.push(k);
            }
        }
        for k in lost {
            if let Granted::Read(_, fs) | Granted::Update(_, fs) = &mut self.grants[k].privilege {
                fs.ids.retain(|&x| x != f);
            }
        }
    }

    pub(super) fn rekey(
        &mut self,
        run: &Run,
        relation: &QName,
        key: &[ast::Ident],
        carry: &[ast::FieldPath],
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
        let resolving = Resolving::new(self, run.module());
        let r = resolving.relation(relation)?;
        let rel = schema
            .ids()
            .relation_of(CatRelId::from_raw(r))
            .expect("a held relation variable");
        let rname = schema.relation(rel).name.clone();
        let needed = carried_fields(schema, rel);
        let mut listed = Vec::new();
        let mut fields = Vec::new();
        for p in carry {
            let def = self
                .def_in(schema, resolving.ty(&p.ty)?)
                .expect("a held type");
            let entry = (
                def,
                p.ctor.as_ref().map(|c| c.text.clone()),
                p.field.text.clone(),
            );
            if !needed.contains(&entry) {
                return refuse_at(
                    p.field.span,
                    format!(
                        "`{}` is not a field that references `{rname}`",
                        path_text(p)
                    ),
                );
            }
            let (t, c) = resolving.part(p)?;
            fields.push(self.field_of(t, c, &p.field.text).expect("a carried field"));
            listed.push(entry);
        }
        if let Some((def, ctor, f)) = needed.iter().find(|e| !listed.contains(e)) {
            let t = &schema.ty(*def).name;
            let p = match ctor {
                Some(c) => format!("{t}.{c}.{f}"),
                None => format!("{t}.{f}"),
            };
            return refuse(format!(
                "`rekey relation {rname}` is refused: field `{p}` references `{rname}` and is not carried"
            ));
        }
        let maps = Ok(vec![RowMap::Rekey { relation: rel }]);
        let names: Vec<String> = key.iter().map(|x| x.text.clone()).collect();
        let printed = |ts: &[Term]| ts.iter().map(|t| t.print(self)).collect::<Vec<_>>();
        let same = self.relations[&r].clauses.iter().find_map(|c| match c {
            ClauseM::Key { id, fields, .. } if printed(fields) == names => Some(*id),
            _ => None,
        });
        let held = self.relations.get_mut(&r).expect("the relation variable");
        let Some(clause) = held
            .clauses
            .iter_mut()
            .find(|c| matches!(c, ClauseM::Key { .. }))
        else {
            return refuse(format!("`{rname}` declares no key"));
        };
        if let ClauseM::Key {
            id,
            fields: written,
            listed,
            generate,
        } = clause
        {
            *id = same.flatten();
            *written = key.iter().map(|x| Term::text(&x.text)).collect();
            *listed = key.len() > 1;
            *generate = None;
        }
        let (Some(first), Some(last)) = (key.first(), key.last()) else {
            return broken("a rekey to no key");
        };
        let key_id = match self.build_adopting(&[], &run.ahead(self)) {
            Ok(after) => {
                self.relift(&after, Item::Relation(r))?;
                Self::key_id(&after, r)?
            }
            Err(_) => {
                if !self.unlifted.contains(&Item::Relation(r)) {
                    self.unlifted.push(Item::Relation(r));
                }
                self.rekeyed_unbuilt.insert(r);
                run.late(Late::Key { relation: r });
                0
            }
        };
        Ok((
            Resolved::Rekey {
                relation: r,
                key: key_id,
                fields: canonical_term(run.text(), first.span.to(last.span), &Names::default())
                    .map_err(|u| Fail::Broken(format!("a key does not resolve: {u:?}")))?,
            },
            fields
                .into_iter()
                .map(|field| Clause::Carry { field })
                .collect(),
            maps,
        ))
    }

    /// Returns the id of the key of relation variable `r` in `built`.
    pub(super) fn key_id(built: &Built, r: u64) -> Res<u64> {
        let ids = built.schema().ids();
        match ids
            .relation_of(CatRelId::from_raw(r))
            .and_then(|rel| ids.key_id(rel))
        {
            Some(k) => Ok(k.raw()),
            None => broken("a rekeyed relation variable without a key id"),
        }
    }
}
