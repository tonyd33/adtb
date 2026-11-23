use std::ops::Range;

use super::split::kind_word;
use super::{carried, ctor_in, def_in, sel_idents};
use crate::ast::{self, CtorSel};
use crate::check::{self, Ref};
use crate::evolve::carry::{self, Structural};
use crate::evolve::ctor::{Carries, arm_image, element_head};
use crate::evolve::model::field::Applied;
use crate::evolve::model::op::{Fail, Res, broken, refuse, refuse_at};
use crate::evolve::model::reselect::{Edit, Mode, sel_range};
use crate::evolve::model::run::{Built, Late, Run};
use crate::evolve::model::*;
use crate::evolve::{Clause, Merged, Resolved, RowMap};
use crate::ids::{CatCtorId, CatFieldId};
use crate::schema::{FieldDef, Schema, TypeDef, TypeDefKind};
use crate::span::{FileId, Span};
use crate::types::Ty;

/// What a merge replaces and makes, for rewriting what ranges over its sources.
struct Image<'a> {
    /// The merged constructor's name.
    ctor: &'a str,
    disc: &'a Disc,
    /// The sources' names, in declaration order.
    sources: &'a [String],
    /// The sources' positions in the type, in declaration order.
    src_ix: &'a [u32],
    carries: &'a Carries,
}

/// The fields of a merged row that tell which merged constructor it came from, with each
/// constructor's values of them as expression text.
struct Disc {
    fields: Vec<String>,
    values: BTreeMap<String, Vec<(String, String)>>,
}

impl Disc {
    /// Returns the discriminator field `field` alone, each of `sources` its own constructor name.
    fn named(field: String, sources: &[String]) -> Disc {
        Disc {
            values: sources
                .iter()
                .map(|s| (s.clone(), vec![(field.clone(), s.clone())]))
                .collect(),
            fields: vec![field],
        }
    }

    fn value(&self, ctor: &str, field: &str) -> Option<&str> {
        self.values
            .get(ctor)?
            .iter()
            .find(|(f, _)| f == field)
            .map(|(_, v)| v.as_str())
    }
}

/// Returns the fields the arms of a derived merge, in `text`, give a value not read from the
/// row, with each source's values.
fn fixed_disc(text: &str, arms: &[ast::Arm]) -> Disc {
    let mut disc = Disc {
        fields: Vec::new(),
        values: BTreeMap::new(),
    };
    for a in arms {
        let (
            Some(ast::PatternKind::Ctor { path, ctor, .. }),
            ast::ExprKind::Ctor {
                record: Some(ast::RecordExpr::Fields(given, _)),
                ..
            },
        ) = (a.patterns.first().map(|p| &p.kind), &a.body.kind)
        else {
            continue;
        };
        let from = ctor.as_ref().unwrap_or(&path.name).text.clone();
        for (g, e) in given {
            if matches!(e.kind, ast::ExprKind::Field(..) | ast::ExprKind::Name(_)) {
                continue;
            }
            if !disc.fields.contains(&g.text) {
                disc.fields.push(g.text.clone());
            }
            disc.values
                .entry(from.clone())
                .or_default()
                .push((g.text.clone(), text[e.span.range()].to_string()));
        }
    }
    disc
}

/// Returns a selection of `names` as written: `C`, or `(C | D)`.
fn sel_text(names: &[String]) -> String {
    match names {
        [one] => one.clone(),
        many => format!("({})", many.join(" | ")),
    }
}

/// A discriminator field's name and the constructor of its sum each merged source takes.
type Discriminator = (String, Vec<CatCtorId>);

/// Returns the one field of `mfields` typed by a declared nullary sum whose constructors are
/// exactly `sources`, in `after`, with the id of the constructor of that sum named as each of
/// `sources`.
fn named_disc(
    after: &Schema,
    mfields: &[FieldDef],
    into: &ast::CtorDef,
    sources: &[String],
) -> Res<Discriminator> {
    let m = &into.name.text;
    let wanted: BTreeSet<&str> = sources.iter().map(String::as_str).collect();
    let discs: Vec<&FieldDef> = mfields
        .iter()
        .filter(|f| match &f.ty {
            Ty::Data { def: d, args, .. } if args.is_empty() => {
                let s = after.ty(*d);
                matches!(s.kind, TypeDefKind::Sum { record: false, .. })
                    && s.ctors().iter().all(|c| c.fields.is_empty())
                    && s.ctors()
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<BTreeSet<_>>()
                        == wanted
            }
            _ => false,
        })
        .collect();
    match discs.as_slice() {
        [one] => {
            let Ty::Data { def, .. } = &one.ty else {
                return broken("a discriminator not typed by a sum");
            };
            let sum = after.ty(*def);
            let tags = sources
                .iter()
                .map(|s| {
                    sum.ctor_index(s)
                        .and_then(|k| sum.ctor_at(k))
                        .and_then(|c| after.ids().ctor_id(c))
                        .ok_or_else(|| {
                            Fail::Broken("a discriminator constructor without an id".into())
                        })
                })
                .collect::<Res<_>>()?;
            Ok((one.name.clone(), tags))
        }
        [] => refuse_at(
            into.name.span,
            format!(
                "the merged constructor `{m}` has no discriminator: a field typed by a nullary sum whose constructors are exactly {}",
                sources.join(", ")
            ),
        ),
        _ => refuse_at(
            into.name.span,
            format!("the merged constructor `{m}` has more than one discriminator field"),
        ),
    }
}

/// Refuse an arm that does not give discriminator `disc` as its own constructor name.
fn own_names(arms: &[ast::Arm], disc: &str) -> Res<()> {
    for arm in arms {
        let ok = match (&arm.patterns.first().map(|p| &p.kind), &arm.body.kind) {
            (
                Some(ast::PatternKind::Ctor { path, ctor, .. }),
                ast::ExprKind::Ctor {
                    record: Some(ast::RecordExpr::Fields(given, _)),
                    ..
                },
            ) => {
                let from = &ctor.as_ref().unwrap_or(&path.name).text;
                given.iter().any(|(g, e)| {
                    g.text == disc
                        && matches!(&e.kind, ast::ExprKind::Ctor { path: p, ctor: None, record: None } if p.name.text == *from)
                })
            }
            _ => false,
        };
        if !ok {
            return refuse_at(
                arm.body.span,
                format!(
                    "the routing gives the discriminator `{disc}` as each row's own constructor name"
                ),
            );
        }
    }
    Ok(())
}

/// Returns what a merge without routing carries: each field of each source to the field of `m`
/// of the same name and type.
fn carried_by_name(
    td: &TypeDef,
    sources: &[String],
    m: &str,
    mfields: &[FieldDef],
    disc: &Disc,
) -> Res<Carries> {
    let mut c = Vec::new();
    for s in sources {
        let k = td.ctor_index(s).expect("a source");
        let sf = &td.ctors()[k as usize].fields;
        for f in sf {
            match mfields
                .iter()
                .find(|g| g.name == f.name && !disc.fields.contains(&g.name))
            {
                Some(g) if g.ty.alike(&f.ty) => {
                    c.push((s.clone(), f.name.clone(), m.to_string(), f.name.clone()));
                }
                _ => {
                    return refuse(format!(
                        "field `{s}.{}` has no counterpart in `{m}`",
                        f.name
                    ));
                }
            }
        }
        for g in mfields {
            if !disc.fields.contains(&g.name) && !sf.iter().any(|f| f.name == g.name) {
                return refuse(format!(
                    "field `{m}.{}` has no counterpart in `{s}`",
                    g.name
                ));
            }
        }
    }
    Ok(c)
}

/// A rewrite of the unique and index clauses of one relation variable: the clauses it removes,
/// the ones it rescopes in place, and those it appends with their text.
#[derive(Default)]
struct Rewrite {
    gone: BTreeSet<usize>,
    rescoped: Vec<(usize, Vec<Term>)>,
    added: Vec<ClauseM>,
}

/// A clause over one merged constructor alone, by its relation variable, kind and elements as
/// the merge maps them.
type Alike = (u64, IndexKind, Vec<String>);

impl Model {
    /// Merge constructors `sel` of `ty` into `into`. With `derived`, the values the routing's arms
    /// fix tell the merged constructors apart, and no field need be a discriminator.
    pub(in crate::evolve::model) fn merge(
        &mut self,
        run: &mut Run,
        (t, sel): (u64, &CtorSel),
        into: &ast::CtorDef,
        arms: Option<&[ast::Arm]>,
        given: &[ast::ArmClause],
        derived: bool,
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
        let sources = self.sources(t, sel)?;
        self.fresh_ctor(t, into)?;
        let m = into.name.text.clone();
        let names: Vec<String> = sources.iter().map(|c| self.ctors[c].name.clone()).collect();
        let def = def_in(schema, t);
        let src_ix: Vec<u32> = sources.iter().map(|&c| ctor_in(schema, c)).collect();
        self.paths_image(t, &sources, "the merge")?;
        let (mfields, disc) = self.merged_shape(run, t, into, &names)?;
        let (disc, tags) = match arms {
            Some(a) if derived => (fixed_disc(run.text(), a), Vec::new()),
            _ => {
                let (field, tags) = disc?;
                (Disc::named(field, &names), tags)
            }
        };
        let carries: Carries = match arms {
            Some(a) => {
                if !derived {
                    own_names(a, &disc.fields[0])?;
                }
                carried(a)
            }
            None => carried_by_name(schema.ty(def), &names, &m, &mfields, &disc)?,
        };
        let stages = self.restated(&built, t, &sources, true)?;
        let c = self.new_ctor(run, t, &m, into.record.as_deref());
        self.defaults_image(&built, t, (&sources, &[c]), "the merge", false)?;
        let merged = match (arms, disc.fields.first()) {
            (None, Some(d)) => Some(self.merged_rows(c, &sources, &src_ix, d, &tags)?),
            _ => None,
        };
        let plan = self.carry_plan(
            run,
            &built,
            Structural::Merge,
            t,
            (&sources, std::slice::from_ref(&m)),
        );
        let mut edits = Vec::new();
        let image = Image {
            ctor: &m,
            disc: &disc,
            sources: &names,
            src_ix: &src_ix,
            carries: &carries,
        };
        edits.extend(self.merged_checks(&built, t, &image)?);
        edits.extend(self.reselect_rows(&built, t, (&sources, &[c]), "the merge", true)?);
        let rewrites = self.merged_clauses(&built, t, &sources, c, &image)?;
        let (arm_edits, written) = self.arm_writes(run, &built, t, (&sources, &sources), given)?;
        edits.extend(arm_edits);
        edits.extend(self.reselect(
            &built,
            t,
            &sources,
            &[c],
            Mode::structural("the merge", true, false),
        )?);
        let routing = match arms {
            Some(a) => Some(self.routing(run, &built, (t, &sources, &[c]), (a, given), true)?),
            None => None,
        };
        let shape = (m.clone(), mfields.iter().map(|f| f.name.clone()).collect());
        let (reaches, routed) = match arms {
            Some(a) => {
                let images: Vec<_> = a.iter().map(arm_image).collect();
                carry::routed(&images, &names, std::slice::from_ref(&shape))
            }
            None => carry::by_name(&carries, &names, &shape),
        };
        let members: Vec<Clause> = sel_idents(sel)
            .iter()
            .filter_map(|x| self.ctor_named(t, &x.text))
            .map(|ctor| Clause::Member { ctor })
            .collect();
        let before = self.clone();
        self.edit_decls(edits)?;
        let rels: Vec<u64> = rewrites.keys().copied().collect();
        for (r, w) in rewrites {
            let rel = self.relations.get_mut(&r).expect("a relation variable");
            for (at, parts) in w.rescoped {
                if let ClauseM::Index { id, scope, .. } = &mut rel.clauses[at] {
                    *id = None;
                    *scope = Some(Scope {
                        listed: parts.len() > 1,
                        parts,
                    });
                }
            }
            let kept: Vec<ClauseM> = rel
                .clauses
                .drain(..)
                .enumerate()
                .filter(|(k, _)| !w.gone.contains(k))
                .map(|(_, x)| x)
                .collect();
            rel.clauses = kept;
            rel.clauses.extend(w.added);
        }
        self.replace_ctors(t, &sources, &[c]);
        self.rebind(&before);
        self.restate(&[c], &stages);
        let file = FileId(self.decl_at(Item::Type(t)).expect("the type") as u32);
        let mut clauses = members;
        let after = self.rebuilt(run, t, &rels)?;
        clauses.extend(self.arm_clauses(run, after.as_ref().map_err(Vec::as_slice), written)?);
        let after = after.ok();
        let record = into.record.is_some();
        let decl = match &after {
            Some(after) => self.ctor_canonical(after, run, file, c, record)?,
            None => run.late(Late::Ctor { ctor: c, record }),
        };
        self.settle_ids(run);
        self.carry_grants(t, plan, (&reaches, &routed), &disc.fields, &[]);
        let map = merged.map(|(discriminator, rows)| RowMap::Merge {
            def,
            into: cat_ctor(c),
            discriminator,
            rows,
        });
        let (routing_text, map) = match routing {
            Some(r) => (
                Some(r.text),
                RowMap::Route {
                    def,
                    from: src_ix,
                    routing: r.rows,
                },
            ),
            None => (None, map.expect("a merge without routing")),
        };
        Ok((
            Resolved::Merge {
                ctor: c,
                decl,
                routing: routing_text,
            },
            clauses,
            Ok(vec![map]),
        ))
    }

    /// Returns the fields merged constructor `into` of type `t` declares, merging constructors
    /// `names`, and its discriminator.
    fn merged_shape(
        &self,
        run: &mut Run,
        t: u64,
        into: &ast::CtorDef,
        names: &[String],
    ) -> Res<(Vec<FieldDef>, Res<Discriminator>)> {
        let mut probe = self.clone();
        let pc = probe.new_ctor(run, t, &into.name.text, into.record.as_deref());
        if let TypeBody::Ctors(cs) = &mut probe.types.get_mut(&t).expect("the type").body {
            cs.push(pc);
        }
        let probed = match probe.built(&[], &run.ahead(&probe)) {
            Ok((b, _)) => b,
            Err(ds) => return refuse(ds[0].diag.message.clone()),
        };
        let ps = probed.schema();
        let mfields: Vec<FieldDef> = ps.ty(def_in(ps, t)).ctors()[ctor_in(ps, pc) as usize]
            .fields
            .clone();
        let disc = named_disc(ps, &mfields, into, names);
        Ok((mfields, disc))
    }

    /// Returns the discriminator field `disc` of merged constructor `c` and how the rows of each
    /// of `sources`, at `src_ix` before the merge, become rows of `c`, the discriminator holding
    /// the tag of `tags` at the source's position.
    fn merged_rows(
        &self,
        c: u64,
        sources: &[u64],
        src_ix: &[u32],
        disc: &str,
        tags: &[CatCtorId],
    ) -> Res<(CatFieldId, Vec<Merged>)> {
        let own = |c: u64| self.ctors[&c].fields.clone().unwrap_or_default();
        let Some(&d) = own(c).iter().find(|f| self.fields[f].name == disc) else {
            return broken("a merged constructor without its discriminator");
        };
        let rows = sources
            .iter()
            .zip(src_ix)
            .zip(tags)
            .map(|((&s, &from), &tag)| {
                let fields = own(c)
                    .into_iter()
                    .filter(|&g| g != d)
                    .filter_map(|g| {
                        let name = &self.fields[&g].name;
                        let f = own(s).into_iter().find(|f| self.fields[f].name == *name)?;
                        Some((cat_field(g), cat_field(f)))
                    })
                    .collect();
                Merged::new(from, tag, fields)
            })
            .collect();
        Ok((cat_field(d), rows))
    }

    /// Returns, by relation variable, how the merge rewrites each unique and index clause over
    /// its sources to range over merged constructor `c`: a clause over every source keeps its
    /// rows; clauses alike over each source alone become one over `c` keyed by the
    /// discriminator; any other clause over some sources refuses.
    fn merged_clauses(
        &self,
        built: &Built,
        t: u64,
        sources: &[u64],
        c: u64,
        image: &Image,
    ) -> Res<BTreeMap<u64, Rewrite>> {
        let header = self.header_names(t);
        let mut out: BTreeMap<u64, Rewrite> = BTreeMap::new();
        let mut alone: BTreeMap<Alike, Vec<(usize, u64, String)>> = BTreeMap::new();
        for cl in self.scoped(built, t, sources, &[c]) {
            let Some(reached) = &cl.reached else { continue };
            let inside: Vec<u64> = reached
                .iter()
                .copied()
                .filter(|x| sources.contains(x))
                .collect();
            if inside.is_empty() {
                continue;
            }
            let shown = format!("{} ({})", kind_word(cl.kind), cl.elems.join(", "));
            let mut fields: Option<Vec<String>> = None;
            for x in &inside {
                let s = &self.ctors[x].name;
                let mut fs = Vec::new();
                for f in &cl.elems {
                    let (head, rest) = element_head(f);
                    let mapped = if header.iter().any(|h| h == head) {
                        Some(f.clone())
                    } else {
                        image
                            .carries
                            .iter()
                            .find(|(a, b, _, _)| a == s && b == head)
                            .map(|(_, _, _, g)| format!("{g}{rest}"))
                    };
                    match mapped {
                        Some(g) => fs.push(g),
                        None => {
                            return refuse(format!(
                                "`{shown}` has no image under the merge: the routing does not carry `{s}.{f}` whole"
                            ));
                        }
                    }
                }
                match &fields {
                    None => fields = Some(fs),
                    Some(prev) if *prev == fs => {}
                    Some(_) => {
                        return refuse(format!(
                            "`{shown}` has no image under the merge: its constructors' fields go to different fields"
                        ));
                    }
                }
            }
            let fields = fields.unwrap_or_default();
            if inside.len() != sources.len() {
                if let ([one], 1) = (inside.as_slice(), reached.len()) {
                    alone
                        .entry((cl.relation, cl.kind, fields))
                        .or_default()
                        .push((cl.at, *one, shown));
                    continue;
                }
                return refuse(format!(
                    "`{shown}` ranges over some but not all of the merged constructors and has no image under the merge"
                ));
            }
            let parts = match cl.rescoped {
                Some(parts) => parts,
                None if fields == cl.elems => continue,
                None => match &self.relations[&cl.relation].clauses[cl.at] {
                    ClauseM::Index { scope: Some(s), .. } => s.parts.clone(),
                    _ => return broken("a scoped clause without a scope"),
                },
            };
            let w = out.entry(cl.relation).or_default();
            if fields == cl.elems {
                w.rescoped.push((cl.at, parts));
                continue;
            }
            w.gone.insert(cl.at);
            w.added.push(ClauseM::Index {
                id: None,
                pending: None,
                kind: cl.kind,
                elems: fields.iter().map(|f| Term::text(f)).collect(),
                scope: Some(Scope {
                    listed: parts.len() > 1,
                    parts,
                }),
            });
        }
        for ((r, kind, fields), held) in alone {
            let each = sources
                .iter()
                .all(|s| held.iter().any(|(_, one, _)| one == s));
            let keyed = kind != IndexKind::Unique || !image.disc.fields.is_empty();
            if !each || !keyed {
                return refuse(format!(
                    "`{} in {}` ranges over some but not all of the merged constructors and has no image under the merge",
                    held[0].2, self.ctors[&held[0].1].name
                ));
            }
            let mut fs = fields;
            if kind == IndexKind::Unique {
                for f in image.disc.fields.iter().rev() {
                    if !fs.contains(f) {
                        fs.insert(0, f.clone());
                    }
                }
            }
            let w = out.entry(r).or_default();
            w.gone.extend(held.iter().map(|(at, _, _)| *at));
            w.added.push(ClauseM::Index {
                id: None,
                pending: None,
                kind,
                elems: fs.iter().map(|f| Term::text(f)).collect(),
                scope: Some(Scope {
                    listed: false,
                    parts: vec![Term::name((Space::Shared, c), image.ctor)],
                }),
            });
        }
        Ok(out)
    }

    /// Returns the edits that carry each check over the merged constructors to the merged one,
    /// as `built` resolves the declarations. A check over some but not all of them becomes a
    /// check matching on the discriminator, with `true` for the others.
    fn merged_checks(&self, built: &Built, t: u64, image: &Image) -> Res<Vec<Edit>> {
        let schema = built.schema();
        let def = def_in(schema, t);
        let td = schema.ty(def);
        let refs = check::schema_refs(schema);
        let rels: Vec<crate::types::RelId> = crate::evolve::Cx::relations_of(schema, def)
            .into_iter()
            .map(|(r, _, _)| r)
            .collect();
        let sets: BTreeSet<&str> = rels
            .iter()
            .flat_map(|&r| schema.relation(r).sets.iter().map(|x| x.name.as_str()))
            .collect();
        let mut edits = Vec::new();
        for chk in &schema.checks {
            let Some(cs) = &chk.ctors else { continue };
            if !rels.contains(&chk.relation) {
                continue;
            }
            let inside = cs.iter().filter(|c| image.src_ix.contains(c)).count();
            if inside == 0 {
                continue;
            }
            let partial = inside < image.src_ix.len();
            if partial && inside < cs.len() {
                return refuse(format!(
                    "check `{}` has no image under the merge; drop or rewrite it in this or an earlier migration",
                    chk.name
                ));
            }
            let j = chk.file.0 as usize;
            let Some(ast::Decl::Check(cd)) = built.decl(chk.file) else {
                continue;
            };
            let Some(sel) = &cd.on.sel else { continue };
            let h = format!("module {}\n", self.decls[j].module).len();
            let p = cd.predicate.span;
            let pred = p.start as usize - h..p.end as usize - h;
            let text = &built.text(chk.file)[h..];
            let body =
                self.carried_predicate((def, td), &refs, image, chk, (text, pred.clone(), h))?;
            let idents: Vec<ast::Ident> = sel_idents(sel).into_iter().cloned().collect();
            if !idents.iter().any(|x| sets.contains(x.text.as_str())) {
                let mut names: Vec<String> = Vec::new();
                for &c in cs {
                    let n = if image.src_ix.contains(&c) {
                        image.ctor.to_string()
                    } else {
                        td.ctors()[c as usize].name.clone()
                    };
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                let r = sel_range(built.text(chk.file), &idents);
                edits.push(Edit::new(
                    j,
                    r.start - h..r.end - h,
                    Term::text(&sel_text(&names)),
                ));
            }
            if !partial {
                edits.push(Edit::new(j, pred, Term::text(&body)));
                continue;
            }
            let mut nested = false;
            crate::visit::each_expr(&cd.predicate, &mut |e| {
                nested |= matches!(e.kind, ast::ExprKind::Match(..));
            });
            let body = if nested { format!("({body})") } else { body };
            let untold = || {
                refuse(format!(
                    "check `{}` has no image under the merge: no one field of `{}` tells its constructors apart",
                    chk.name, image.ctor
                ))
            };
            let [field] = image.disc.fields.as_slice() else {
                return untold();
            };
            let mut arms: Vec<String> = Vec::new();
            for (s, c) in image.sources.iter().zip(image.src_ix) {
                let Some(v) = image.disc.value(s, field) else {
                    return untold();
                };
                let e = if cs.contains(c) {
                    body.as_str()
                } else {
                    "true"
                };
                arms.push(format!("{v} -> {e}"));
            }
            edits.push(Edit::new(
                j,
                pred,
                Term::text(&format!("match {field} | {}", arms.join(" | "))),
            ));
        }
        Ok(edits)
    }

    /// Returns the predicate `pred` of check `chk`, in `text` whose header is `h` long, with each
    /// field of a merged constructor renamed to the field the routing carries it to whole.
    fn carried_predicate(
        &self,
        (def, td): (crate::types::TypeId, &TypeDef),
        refs: &[(FileId, Span, Ref)],
        image: &Image,
        chk: &crate::schema::CheckDef,
        (text, pred, h): (&str, Range<usize>, usize),
    ) -> Res<String> {
        let refused = |why: String| {
            refuse(format!(
                "check `{}` has no image under the merge: {why}",
                chk.name
            ))
        };
        let cs = chk.ctors.as_deref().unwrap_or_default();
        let mut renames: Vec<(Range<usize>, String)> = Vec::new();
        for (file, span, r) in refs {
            let Ref::Field {
                def: d, name, pun, ..
            } = r
            else {
                continue;
            };
            if *file != chk.file || td.field(None, name).is_some() {
                continue;
            }
            let at = span.start as usize - h..span.end as usize - h;
            if !(pred.start <= at.start && at.end <= pred.end) || *d != def {
                continue;
            }
            let mut to: Option<&str> = None;
            for &c in cs {
                let s = &td.ctors()[c as usize].name;
                let carried = if image.src_ix.contains(&c) {
                    image
                        .carries
                        .iter()
                        .find(|(x, y, _, _)| x == s && y == name)
                        .map(|(_, _, _, g)| g.as_str())
                } else {
                    Some(name.as_str())
                };
                match (carried, to) {
                    (None, _) => {
                        return refused(format!("the routing does not carry `{s}.{name}` whole"));
                    }
                    (Some(g), Some(prev)) if g != prev => {
                        return refused(format!(
                            "its constructors' field `{name}` goes to different fields"
                        ));
                    }
                    (Some(g), _) => to = Some(g),
                }
            }
            if let Some(to) = to
                && to != name
            {
                let new = if *pun {
                    format!("{name}: {to}")
                } else {
                    to.to_string()
                };
                renames.push((at.start - pred.start..at.end - pred.start, new));
            }
        }
        renames.sort_by_key(|(r, _)| r.start);
        let old = &text[pred];
        let mut body = String::new();
        let mut from = 0;
        for (r, new) in &renames {
            body.push_str(&old[from..r.start]);
            body.push_str(new);
            from = r.end;
        }
        body.push_str(&old[from..]);
        Ok(body)
    }
}
