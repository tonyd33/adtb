use std::collections::BTreeSet;

use super::super::{Action, Clause, Constraint, Ctor, Field, Rel, Scope, Stage, Ty, literal};
use super::rows::{Effect, literal_value};
use super::{AGGREGATE, Func, Kind, Model, Op, ROLE, broken, data, emptying, fill, op};
use crate::Rng;

impl Model {
    pub(super) fn stage(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let mut all = Vec::new();
        for i in 0..self.s.rels.len() {
            for c in 0..self.rel(i).ctors.len() {
                all.push((i, c));
            }
        }
        let st = |m: &Model, (i, c): (usize, usize)| m.rel(i).states[c];
        let verb = match kind {
            Kind::Enable => "enable",
            Kind::Disable => "disable",
            _ => "deprecate",
        };
        let fits: Vec<(usize, usize)> = all
            .iter()
            .copied()
            .filter(|x| {
                let enabled = st(self, *x) == Stage::Enabled;
                match (kind, broke) {
                    (Kind::Enable, false) => !enabled,
                    (Kind::Enable, true) => enabled,
                    (_, false) => enabled && !self.retag_target(x.0, x.1),
                    (_, true) => !enabled || self.retag_target(x.0, x.1),
                }
            })
            .collect();
        let removable: Vec<(usize, usize)> = fits
            .iter()
            .copied()
            .filter(|&(i, c)| kind == Kind::Deprecate && !broke && self.removable(i, c))
            .collect();
        let &(i, c) = match rng.pick_some(&removable) {
            Some(x) if rng.below(4) != 0 => x,
            _ => rng.pick_some(&fits)?,
        };
        let text = format!("{verb} {}", self.ctor_ref(i, c));
        if broke {
            return Some(vec![broken(kind, text)]);
        }
        self.s.rels[i].states[c] = match kind {
            Kind::Enable => Stage::Enabled,
            Kind::Disable => Stage::Declared,
            _ => Stage::Deprecated(self.version),
        };
        Some(vec![if kind == Kind::Disable {
            data(kind, text)
        } else {
            op(kind, text)
        }])
    }

    /// Returns whether a `remove` of constructor `c` of relation `i`, deprecated by an earlier
    /// migration and empty, meets its other preconditions.
    pub(super) fn removable(&self, i: usize, c: usize) -> bool {
        let r = self.rel(i);
        let set_left = r.set.as_ref().is_none_or(|s| s.len() > 1 || s[0] != c);
        let alone = |k: &Clause| k.ctors.as_deref() == Some(&[c][..]);
        r.sum
            && r.ctors.len() > 1
            && set_left
            && !self.selected(i, c)
            && !self.retag_target(i, c)
            && !r.checks.iter().any(alone)
            && !self
                .s
                .constraints
                .iter()
                .any(|k| k.rel == i && k.ctors.contains(&c))
    }

    /// Returns the constructors of sums a structural operator may replace: not the target of a
    /// retag action, named by no clause, constraint or default, and not deprecated.
    pub(super) fn replaceable(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for i in self.sums() {
            for c in 0..self.rel(i).ctors.len() {
                if !self.retag_target(i, c)
                    && !matches!(self.rel(i).states[c], Stage::Deprecated(_))
                {
                    out.push((i, c));
                }
            }
        }
        out
    }

    /// Returns the operators adding, over constructor `c` of relation `i` alone, a clause of each
    /// kind a switch on lets an operator of `kind` replace it under, each added to the model.
    fn seeded(&mut self, i: usize, c: usize, kind: Kind) -> Vec<Op> {
        let r = self.rel(i).clone();
        let fields: Vec<&Field> = r.ctors[c].fields.iter().collect();
        let ints: Vec<&Field> = fields
            .iter()
            .copied()
            .chain(r.plain_header())
            .filter(|f| f.ty == Ty::Int)
            .collect();
        let at = r.target(Some(&vec![c]));
        let mut out = Vec::new();
        let own = fields
            .iter()
            .find(|f| f.ty.keyed() && f.ty != Ty::Bool && r.holding(&f.name, &f.ty) == [c]);
        if super::super::SEARCH_REPLACE_UNDER_UNIQUE
            && let Some(f) = own
            && !r
                .uniques
                .iter()
                .any(|k| k.ctors.as_deref() == Some(&[c][..]))
        {
            let flagged = fields
                .iter()
                .any(|f| f.ty == Ty::Bool && f.action == Action::Unstated);
            if kind == Kind::Split && !flagged {
                let name = self.field_name(i);
                let text = format!(
                    "add field {}: Bool fill true",
                    self.field_path(i, Some(c), &name)
                );
                let effect = Effect::AddField {
                    rel: r.name.clone(),
                    ctor: Some(r.ctors[c].name.clone()),
                    field: name.clone(),
                    value: literal_value(&Ty::Bool, true),
                };
                out.push(op(Kind::AddField, text).with(effect));
                self.s.rels[i].ctors[c]
                    .fields
                    .push(Field::new(&name, Ty::Bool));
            }
            out.push(data(
                Kind::AddUnique,
                format!("add unique ({}) in {at}", f.name),
            ));
            self.s.rels[i].uniques.push(Clause {
                field: f.name.clone(),
                ctors: Some(vec![c]),
                name: String::new(),
            });
        }
        let literal_field = fields
            .iter()
            .find(|f| f.ty.literal() && !r.has_default(c, &f.name));
        if super::super::SEARCH_REPLACE_UNDER_DEFAULT
            && let Some(f) = literal_field
        {
            let v = literal(&f.ty, false);
            let path = format!("{}.{}.{}", r.name, r.ctors[c].name, f.name);
            out.push(op(Kind::AddDefault, format!("add default {path} = {v}")));
            self.s.rels[i].defaults.push((c, f.name.clone(), v));
        }
        let check = super::super::SEARCH_NEST_UNDER_CHECK && kind == Kind::Nest;
        let constraint =
            super::super::SEARCH_MERGE_UNDER_CONSTRAINT && matches!(kind, Kind::Merge | Kind::Nest);
        let int = match ints.first() {
            Some(f) => f.name.clone(),
            None if check || constraint => {
                let name = self.field_name(i);
                let text = format!(
                    "add field {}: I64 fill {}",
                    self.field_path(i, None, &name),
                    literal(&Ty::Int, false)
                );
                let effect = Effect::AddField {
                    rel: r.name.clone(),
                    ctor: None,
                    field: name.clone(),
                    value: literal_value(&Ty::Int, false),
                };
                out.push(op(Kind::AddField, text).with(effect));
                self.s.rels[i].header.push(Field::new(&name, Ty::Int));
                name
            }
            None => String::new(),
        };
        if check {
            let name = self.fresh("k");
            out.push(data(
                Kind::AddCheck,
                format!("add check {name} on {at} ({int} >= (-2))"),
            ));
            self.s.rels[i].checks.push(Clause {
                field: int.clone(),
                ctors: Some(vec![c]),
                name,
            });
        }
        if constraint {
            let k = Constraint {
                rel: i,
                ctors: vec![c],
                field: int,
                name: self.fresh("c"),
            };
            out.push(data(
                Kind::AddConstraint,
                format!("add {}", self.s.constraint_decl(&k)),
            ));
            self.s.constraints.push(k);
        }
        out
    }

    pub(super) fn split(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let names_itself = |m: &Model, (i, c): (usize, usize)| {
            m.rel(i).ctors[c]
                .fields
                .iter()
                .any(|f| match f.ty.target() {
                    Some((t, Scope::Ctors(cs))) => t == i && cs.contains(&c),
                    _ => false,
                })
        };
        let can: Vec<(usize, usize)> = self
            .replaceable()
            .into_iter()
            .filter(|p| super::super::SEARCH_SPLIT_OF_SELF_REFERENCE || !names_itself(self, *p))
            .collect();
        let flagged = |m: &Model, (i, c): (usize, usize)| {
            m.rel(i).ctors[c]
                .fields
                .iter()
                .any(|f| f.ty == Ty::Bool && f.action == Action::Unstated)
        };
        let can: Vec<((usize, usize), bool)> = can
            .into_iter()
            .filter_map(|(i, c)| {
                let one = self.replaces_under(i, &[c], 1, Kind::Split)?;
                let two = flagged(self, (i, c))
                    && self.replaces_under(i, &[c], 2, Kind::Split) == Some(true);
                let seeds = !self.clone().seeded(i, c, Kind::Split).is_empty();
                Some(((i, c), one || two || seeds))
            })
            .collect();
        let can = Model::steered(can);
        let &(i, c) = rng.pick_some(&can)?;
        let mut out = if broke {
            Vec::new()
        } else {
            self.seeded(i, c, Kind::Split)
        };
        let steer = self.replaces_under(i, &[c], 2, Kind::Split) == Some(true);
        let path = self.ctor_path(i, c);
        let src = self.rel(i).ctors[c].clone();
        let decl = |m: &Model, fs: &[&Field]| -> String {
            let fs: Vec<String> = fs.iter().map(|f| m.s.field_decl(m.rel(i), f)).collect();
            if fs.is_empty() {
                String::new()
            } else {
                format!(" {{ {} }}", fs.join(", "))
            }
        };
        let carried = |fs: &[&Field]| -> String {
            let fs: Vec<String> = fs.iter().map(|f| format!("{0}: c.{0}", f.name)).collect();
            format!(" {{ {} }}", fs.join(", "))
        };
        let state = self.rel(i).states[c];
        let flag = src
            .fields
            .iter()
            .position(|f| f.ty == Ty::Bool && f.action == Action::Unstated);
        let products = match flag {
            Some(b)
                if (steer || rng.below(2) == 0)
                    && self.replaces_under(i, &[c], 2, Kind::Split).is_some() =>
            {
                2 + b
            }
            _ => 1,
        };
        let names: Vec<String> = if broke {
            let r = self.rel(i);
            vec![r.ctors[(c + 1) % r.ctors.len()].name.clone()]
        } else if products == 1 {
            vec![self.ctor_name(i)]
        } else {
            let a = self.ctor_name(i);
            vec![a, self.ctor_name(i)]
        };
        if broke && names[0] == src.name {
            return None;
        }
        let (text, made) = if products == 1 {
            let fs: Vec<&Field> = src.fields.iter().collect();
            let text = format!(
                "split constructor {path} into {}{}\n  by match\n    | {} c -> {}{}",
                names[0],
                decl(self, &fs),
                src.name,
                names[0],
                carried(&fs)
            );
            let made = vec![Ctor {
                name: names[0].clone(),
                fields: src.fields.clone(),
            }];
            (text, made)
        } else {
            let b = products - 2;
            let flag = src.fields[b].name.clone();
            let kept: Vec<&Field> = src.fields.iter().filter(|f| f.name != flag).collect();
            let arm = |name: &str, v: bool| {
                let fs: Vec<String> = kept.iter().map(|f| f.name.clone()).collect();
                let pattern = if fs.is_empty() {
                    format!("{{ {flag}: {v} }}")
                } else {
                    format!("{{ {flag}: {v}, {} }}", fs.join(", "))
                };
                let carry: Vec<String> = kept.iter().map(|f| format!("{0}: {0}", f.name)).collect();
                format!(
                    "\n    | {} {pattern} -> {name} {{ {} }}",
                    src.name,
                    carry.join(", ")
                )
            };
            let text = format!(
                "split constructor {path} into {}{} | {}{}\n  by match{}{}",
                names[0],
                decl(self, &kept),
                names[1],
                decl(self, &kept),
                arm(&names[0], true),
                arm(&names[1], false)
            );
            let fields: Vec<Field> = kept.into_iter().cloned().collect();
            let made = names
                .iter()
                .map(|n| Ctor {
                    name: n.clone(),
                    fields: fields.clone(),
                })
                .collect();
            (text, made)
        };
        if broke {
            return Some(vec![broken(Kind::Split, text)]);
        }
        let effect = Effect::Split {
            rel: self.rel(i).name.clone(),
            ctor: src.name.clone(),
            flag: (products > 1).then(|| src.fields[products - 2].name.clone()),
            products: names,
        };
        let states = vec![state; made.len()];
        self.replace(i, &[c], made, states);
        out.push(op(Kind::Split, text).with(effect));
        Some(out)
    }

    pub(super) fn merge(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let free = self.replaceable();
        let mut pairs = Vec::new();
        for &(i, c) in &free {
            for &(j, d) in &free {
                if i != j || c >= d {
                    continue;
                }
                let r = self.rel(i);
                let (a, b) = (&r.ctors[c], &r.ctors[d]);
                let alike = |x: &Ctor, y: &Ctor| {
                    x.fields.len() == y.fields.len()
                        && x.fields
                            .iter()
                            .all(|f| y.field(&f.name).is_some_and(|g| g.ty == f.ty))
                };
                let fits = match kind {
                    Kind::Merge => alike(a, b),
                    _ => a.fields.is_empty() && b.fields.is_empty(),
                };
                let set = r.set.clone().unwrap_or_default();
                let apart = set.contains(&c) != set.contains(&d)
                    || self.refs().iter().any(|(_, _, f)| match f.ty.target() {
                        Some((t, Scope::Ctors(cs))) => t == i && cs.contains(&c) != cs.contains(&d),
                        _ => false,
                    });
                let actions = a
                    .fields
                    .iter()
                    .chain(&b.fields)
                    .any(|f| matches!(f.action, Action::Retag | Action::Clear));
                let staged = r.states[c] == r.states[d] || super::super::SEARCH_MERGE_ACROSS_STAGES;
                let under = self.replaces_under(i, &[c, d], 1, kind);
                if fits
                    && !apart
                    && !actions
                    && staged
                    && let Some(switched) = under
                {
                    pairs.push(((i, c, d), switched));
                }
            }
        }
        let mut out = Vec::new();
        let pairs = Model::steered(pairs);
        let (i, c, d) = match rng.pick_some(&pairs) {
            Some(p) => *p,
            None => {
                let i = *rng.pick_some(&self.sums())?;
                let ty = self.rel(i).ty.clone();
                let at = self.rel(i).ctors.len();
                for _ in 0..2 {
                    let name = self.ctor_name(i);
                    out.push(op(
                        Kind::AddConstructor,
                        format!("add constructor {ty}.{name}"),
                    ));
                    let r = &mut self.s.rels[i];
                    r.ctors.push(Ctor {
                        name,
                        fields: Vec::new(),
                    });
                    r.states.push(Stage::Declared);
                }
                (i, at, at + 1)
            }
        };
        if !broke {
            let seeded = self.seeded(i, c, kind);
            out.extend(seeded);
        }
        let ty = self.rel(i).ty.clone();
        let (cn, dn) = (
            self.rel(i).ctors[c].name.clone(),
            self.rel(i).ctors[d].name.clone(),
        );
        let tag = self.fresh("K");
        let product = self.ctor_name(i);
        let tag_ctors = if broke && kind == Kind::Nest {
            vec![cn.clone(), "Zz".to_owned()]
        } else {
            vec![cn.clone(), dn.clone()]
        };
        let add_type = op(
            Kind::AddType,
            format!("add type {tag} = {}", tag_ctors.join(" | ")),
        );
        let fields = self.rel(i).ctors[c].fields.clone();
        let field = self.fresh(if kind == Kind::Merge { "k" } else { "f" });
        let mut decl: Vec<String> = Vec::new();
        if !(broke && kind == Kind::Merge) {
            decl.push(format!("{field}: {tag}"));
        }
        decl.extend(fields.iter().map(|f| self.s.field_decl(self.rel(i), f)));
        let verb = if kind == Kind::Merge { "merge" } else { "nest" };
        let text = format!(
            "{verb} constructor {ty}.({cn} | {dn}) into {product} {{ {} }}",
            decl.join(", ")
        );
        if broke {
            out.extend([add_type, broken(kind, text)]);
            return Some(out);
        }
        let effect = Effect::Merge {
            rel: self.rel(i).name.clone(),
            sources: vec![cn, dn],
            product: product.clone(),
            field: field.clone(),
        };
        let apart = d != c + 1 && !super::super::SEARCH_REVERTED_MERGE_APART;
        let states = &self.rel(i).states;
        let declared = states[c] == Stage::Declared || states[d] == Stage::Declared;
        let state = if declared {
            Stage::Declared
        } else {
            Stage::Enabled
        };
        let mut fs = vec![Field::new(&field, Ty::Tag(tag.clone(), tag_ctors.clone()))];
        fs.extend(fields);
        self.s.extra.types.push((tag, tag_ctors));
        self.replace(
            i,
            &[c, d],
            vec![Ctor {
                name: product,
                fields: fs,
            }],
            vec![state],
        );
        let merged = op(kind, text).with(effect);
        out.extend([add_type, if apart { merged.inexact() } else { merged }]);
        Some(out)
    }

    pub(super) fn unnest(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let mut out = Vec::new();
        let mut can = self.unnestable();
        if can.is_empty() && !broke {
            out = self.merge(rng, Kind::Nest, false)?;
            can = self.unnestable();
        }
        if broke {
            let mut not = Vec::new();
            for i in self.sums() {
                for c in 0..self.rel(i).ctors.len() {
                    if !matches!(self.rel(i).ctors[c].fields.as_slice(), [f] if matches!(f.ty, Ty::Tag(..)))
                    {
                        not.push((i, c));
                    }
                }
            }
            let &(i, c) = rng.pick_some(&not)?;
            let text = format!("unnest constructor {}", self.ctor_path(i, c));
            return Some(vec![broken(Kind::Unnest, text)]);
        }
        let &(i, c) = rng.pick_some(&can)?;
        let text = format!("unnest constructor {}", self.ctor_path(i, c));
        let Ty::Tag(_, cs) = self.rel(i).ctors[c].fields[0].ty.clone() else {
            unreachable!("an unnested constructor holds a sum");
        };
        let effect = Effect::Unnest {
            rel: self.rel(i).name.clone(),
            ctor: self.rel(i).ctors[c].name.clone(),
            field: self.rel(i).ctors[c].fields[0].name.clone(),
            ctors: cs.clone(),
        };
        let state = self.rel(i).states[c];
        let made: Vec<Ctor> = cs
            .iter()
            .map(|n| Ctor {
                name: n.clone(),
                fields: Vec::new(),
            })
            .collect();
        let states = vec![state; made.len()];
        self.replace(i, &[c], made, states);
        out.push(op(Kind::Unnest, text).with(effect));
        Some(out)
    }

    /// Returns each constructor holding one field of a sum declared by a nest, whose
    /// constructors are free names of its relation.
    pub(super) fn unnestable(&self) -> Vec<(usize, usize)> {
        let mut can = Vec::new();
        for (i, c) in self.replaceable() {
            let ctor = &self.rel(i).ctors[c];
            if let [f] = ctor.fields.as_slice()
                && let Ty::Tag(_, cs) = &f.ty
                && cs
                    .iter()
                    .all(|n| self.rel(i).ctors.iter().all(|d| d.name != *n))
                && cs.iter().all(|n| *n != self.rel(i).set_name)
                && self
                    .replaces_under(i, &[c], cs.len(), Kind::Unnest)
                    .is_some()
            {
                can.push((i, c));
            }
        }
        can
    }

    /// Returns every constructor field of the schema: relation, constructor, field.
    pub(super) fn own_fields(&self) -> Vec<(usize, usize, Field)> {
        let mut out = Vec::new();
        for (i, r) in self.s.rels.iter().enumerate() {
            for (c, ctor) in r.ctors.iter().enumerate() {
                out.extend(ctor.fields.iter().map(|f| (i, c, f.clone())));
            }
        }
        out
    }

    pub(super) fn add_field(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let i = rng.below(self.s.rels.len() as u64) as usize;
        let c = rng.below(self.rel(i).ctors.len() as u64) as usize;
        if broke {
            let f = match self.rel(i).ctors[c].fields.first() {
                Some(f) => f.name.clone(),
                None => self
                    .rel(i)
                    .header
                    .first()
                    .map_or("id".into(), |f| f.name.clone()),
            };
            let text = format!("add field {}: I64 fill 0", self.field_path(i, Some(c), &f));
            return Some(vec![broken(Kind::AddField, text)]);
        }
        if self.retag_target(i, c) {
            return None;
        }
        let name = self.field_name(i);
        let ty = rng.pick(&[Ty::Int, Ty::Text, Ty::Bool, Ty::OptInt]).clone();
        let other = rng.below(2) == 0;
        let text = format!(
            "add field {}: {} fill {}",
            self.field_path(i, Some(c), &name),
            self.s.ty_name(&ty),
            literal(&ty, other)
        );
        let effect = Effect::AddField {
            rel: self.rel(i).name.clone(),
            ctor: Some(self.rel(i).ctors[c].name.clone()),
            field: name.clone(),
            value: literal_value(&ty, other),
        };
        self.s.rels[i].ctors[c].fields.push(Field::new(&name, ty));
        Some(vec![op(Kind::AddField, text).with(effect)])
    }

    /// Returns whether field `f` of constructor `c` of relation `i` may go without breaking
    /// what names it.
    pub(super) fn field_free(&self, i: usize, c: usize, f: &Field) -> bool {
        !self.clause_names_field(i, Some(c), &f.name)
            && !self.retag_target(i, c)
            && f.action != Action::Retag
            && !self.s.extra.computed.iter().any(|(r, _, _)| *r == i)
    }

    pub(super) fn drop_field(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        if broke {
            let i = rng.below(self.s.rels.len() as u64) as usize;
            let text = format!("drop field {}", self.field_path(i, Some(0), "zz"));
            return Some(vec![broken(Kind::DropField, text)]);
        }
        let can: Vec<(usize, usize, Field)> = self
            .own_fields()
            .into_iter()
            .filter(|(i, c, f)| self.field_free(*i, *c, f))
            .collect();
        let (i, c, f) = rng.pick_some(&can)?.clone();
        let text = format!("drop field {}", self.field_path(i, Some(c), &f.name));
        let effect = Effect::DropField {
            rel: self.rel(i).name.clone(),
            ctor: Some(self.rel(i).ctors[c].name.clone()),
            field: f.name.clone(),
        };
        self.s.rels[i].ctors[c].fields.retain(|g| g.name != f.name);
        Some(vec![op(Kind::DropField, text).with(effect)])
    }

    pub(super) fn rename_field(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let fields = self.own_fields();
        let (i, c, f) = rng.pick_some(&fields)?.clone();
        let path = self.field_path(i, Some(c), &f.name);
        if broke {
            let r = self.rel(i);
            let taken = r.ctors[c]
                .fields
                .iter()
                .find(|g| g.name != f.name)
                .or(r.header.first())?
                .name
                .clone();
            return Some(vec![broken(
                Kind::RenameField,
                format!("rename field {path} to {taken}"),
            )]);
        }
        let r = self.rel(i);
        let shared = |k: &Clause| {
            k.field == f.name
                && k.ctors
                    .as_ref()
                    .is_none_or(|cs| cs.len() > 1 && cs.contains(&c))
        };
        if r.uniques
            .iter()
            .chain(&r.indexes)
            .chain(&r.checks)
            .any(shared)
            || !r.sum
        {
            return None;
        }
        let to = if super::super::SEARCH_RENAME_TO_BOUND {
            super::super::BOUND.to_owned()
        } else {
            self.field_name(i)
        };
        let effect = Effect::RenameField {
            rel: self.rel(i).name.clone(),
            ctor: self.rel(i).ctors[c].name.clone(),
            from: f.name.clone(),
            to: to.clone(),
        };
        let r = &mut self.s.rels[i];
        for g in &mut r.ctors[c].fields {
            if g.name == f.name {
                g.name.clone_from(&to);
            }
        }
        for k in r
            .uniques
            .iter_mut()
            .chain(&mut r.indexes)
            .chain(&mut r.checks)
        {
            if k.field == f.name && k.ctors.as_deref() == Some(&[c][..]) {
                k.field.clone_from(&to);
            }
        }
        for (d, g, _) in &mut r.defaults {
            if *d == c && *g == f.name {
                g.clone_from(&to);
            }
        }
        for k in &mut self.s.constraints {
            if k.rel == i && k.ctors.contains(&c) && k.field == f.name {
                k.field.clone_from(&to);
            }
        }
        Some(vec![
            op(Kind::RenameField, format!("rename field {path} to {to}")).with(effect),
        ])
    }

    pub(super) fn move_up(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let mut can = Vec::new();
        let mut shared = Vec::new();
        for (i, c, f) in self.own_fields() {
            let r = self.rel(i);
            if !r.sum || r.ctors.len() < 2 {
                continue;
            }
            let others = (0..r.ctors.len()).any(|d| d != c && r.ctors[d].field(&f.name).is_some());
            if others {
                if fill(&f.ty).is_some() {
                    shared.push((i, c, f));
                }
            } else if fill(&f.ty).is_some()
                && f.action == Action::Unstated
                && f.ty.target().is_none()
                && self.field_free(i, c, &f)
            {
                can.push((i, c, f));
            }
        }
        let pool = if broke { &shared } else { &can };
        let (i, c, f) = rng.pick_some(pool)?.clone();
        let r = self.rel(i);
        let arms: Vec<String> = (0..r.ctors.len())
            .filter(|d| *d != c)
            .map(|d| {
                format!(
                    "\n  | {} x -> {}",
                    r.ctors[d].name,
                    fill(&f.ty).expect("a fillable field")
                )
            })
            .collect();
        let text = format!(
            "move field {} to {} fill match{}",
            self.field_path(i, Some(c), &f.name),
            r.ty,
            arms.concat()
        );
        if broke {
            return Some(vec![broken(Kind::MoveFieldUp, text)]);
        }
        let effect = Effect::Fill {
            rel: r.name.clone(),
            except: r.ctors[c].name.clone(),
            field: f.name.clone(),
            value: literal_value(&f.ty, false),
        };
        let r = &mut self.s.rels[i];
        r.ctors[c].fields.retain(|g| g.name != f.name);
        r.header.push(f);
        Some(vec![op(Kind::MoveFieldUp, text).with(effect)])
    }

    pub(super) fn move_down(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        if broke {
            let i = *rng.pick_some(&self.sums())?;
            let r = self.rel(i);
            let own = r
                .ctors
                .iter()
                .flat_map(|c| &c.fields)
                .find(|f| r.header.iter().all(|h| h.name != f.name))?;
            let text = format!(
                "move field {}.{} to {}.{}",
                r.ty, own.name, r.ty, r.ctors[0].name
            );
            return Some(vec![broken(Kind::MoveFieldDown, text)]);
        }
        let mut can = Vec::new();
        for i in self.sums() {
            let r = self.rel(i);
            for f in r.plain_header() {
                let to: Vec<usize> = (0..r.ctors.len())
                    .filter(|c| r.ctors[*c].field(&f.name).is_none() && !self.retag_target(i, *c))
                    .collect();
                let acted = !matches!(f.action, Action::Unstated | Action::Restrict)
                    && !super::super::SEARCH_MOVED_DOWN_ACTION;
                if !to.is_empty() && !acted && !self.clause_names_field(i, None, &f.name) {
                    can.push((i, f.clone(), to));
                }
            }
        }
        let (i, f, all) = rng.pick_some(&can)?.clone();
        let mut to: Vec<usize> = all.iter().copied().filter(|_| rng.below(2) == 0).collect();
        if to.is_empty() {
            to.push(*rng.pick(&all));
        }
        let r = self.rel(i);
        let text = format!(
            "move field {}.{} to {}.{}",
            r.ty,
            f.name,
            r.ty,
            r.ctor_set(&to)
        );
        let effect = Effect::MoveDown {
            rel: r.name.clone(),
            field: f.name.clone(),
            to: to.iter().map(|c| r.ctors[*c].name.clone()).collect(),
        };
        let r = &mut self.s.rels[i];
        r.header.retain(|g| g.name != f.name);
        for c in to {
            r.ctors[c].fields.push(f.clone());
        }
        Some(vec![op(Kind::MoveFieldDown, text).with(effect)])
    }

    pub(super) fn alter_field(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        if broke {
            let ints: Vec<(usize, usize, Field)> = self
                .own_fields()
                .into_iter()
                .filter(|(_, _, f)| f.ty == Ty::Int)
                .collect();
            let (i, c, f) = rng.pick_some(&ints)?.clone();
            let text = format!(
                "alter field {} on delete cascade",
                self.field_path(i, Some(c), &f.name)
            );
            return Some(vec![broken(Kind::AlterField, text)]);
        }
        let mut can = Vec::new();
        for (i, r) in self.s.rels.iter().enumerate() {
            for f in r.plain_header() {
                can.push((i, None, f.clone()));
            }
            for (c, ctor) in r.ctors.iter().enumerate() {
                can.extend(ctor.fields.iter().map(|f| (i, Some(c), f.clone())));
            }
        }
        can.retain(|(_, _, f)| f.ty.target().is_some() && f.action != Action::Retag);
        let (i, c, f) = rng.pick_some(&can)?.clone();
        let mut actions = vec![Action::Restrict, Action::Cascade];
        if matches!(f.ty, Ty::OptRef(..)) {
            actions.push(Action::Clear);
        }
        actions.retain(|a| *a != f.action);
        let a = *rng.pick(&actions);
        let word = match a {
            Action::Restrict => "restrict",
            Action::Cascade => "cascade",
            _ => "clear",
        };
        let text = format!(
            "alter field {} on delete {word}",
            self.field_path(i, c, &f.name)
        );
        let r = &mut self.s.rels[i];
        let fields = match c {
            Some(c) => &mut r.ctors[c].fields,
            None => &mut r.header,
        };
        for g in fields.iter_mut().filter(|g| g.name == f.name) {
            g.action = a;
        }
        Some(vec![op(Kind::AlterField, text)])
    }

    pub(super) fn add_clause(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let i = rng.below(self.s.rels.len() as u64) as usize;
        let verb = if kind == Kind::AddUnique {
            "unique"
        } else {
            "index"
        };
        let r = self.rel(i);
        if broke {
            let text = format!("add {verb} (zz) in {}", r.name);
            return Some(vec![broken(kind, text)]);
        }
        let mut fields: Vec<(String, Option<Vec<usize>>)> = r
            .plain_header()
            .iter()
            .filter(|f| f.ty.keyed())
            .map(|f| (f.name.clone(), None))
            .collect();
        for c in &r.ctors {
            for f in c.fields.iter().filter(|f| f.ty.keyed()) {
                let holding = r.holding(&f.name, &f.ty);
                let shared = holding.len() > 1 && !super::super::SEARCH_CLAUSE_OVER_SHARED_FIELD;
                let scope = (r.sum).then_some(holding);
                if !shared && !fields.iter().any(|(n, s)| *n == f.name && *s == scope) {
                    fields.push((f.name.clone(), scope));
                }
            }
        }
        let held = if kind == Kind::AddUnique {
            &r.uniques
        } else {
            &r.indexes
        };
        let same = |a: &Option<Vec<usize>>, b: &Option<Vec<usize>>| {
            let set = |x: &Option<Vec<usize>>| {
                x.as_ref()
                    .map(|cs| cs.iter().copied().collect::<BTreeSet<usize>>())
            };
            set(a) == set(b)
        };
        let (field, ctors) = rng.pick_some(&fields)?.clone();
        let duplicate = held
            .iter()
            .any(|c| c.field == field && same(&c.ctors, &ctors))
            && !super::super::SEARCH_DUPLICATE_CLAUSE;
        let clause = Clause {
            field: field.clone(),
            ctors: ctors.clone(),
            name: String::new(),
        };
        let text = format!("add {verb} ({field}) in {}", r.target(ctors.as_ref()));
        let r = &mut self.s.rels[i];
        let added = if kind == Kind::AddUnique {
            r.uniques.push(clause);
            data(kind, text)
        } else {
            r.indexes.push(clause);
            op(kind, text)
        };
        Some(vec![if duplicate { added.inexact() } else { added }])
    }

    pub(super) fn drop_clause(
        &mut self,
        rng: &mut Rng,
        kind: Kind,
        broke: bool,
    ) -> Option<Vec<Op>> {
        let unique = kind == Kind::DropUnique;
        let verb = if unique { "unique" } else { "index" };
        if broke {
            let r = rng.pick(&self.s.rels);
            return Some(vec![broken(
                kind,
                format!("drop {verb} (zz) in {}", r.name),
            )]);
        }
        let mut can = Vec::new();
        for (i, r) in self.s.rels.iter().enumerate() {
            let clauses = if unique { &r.uniques } else { &r.indexes };
            let twice = |k: usize| {
                let same = |c: &Clause| {
                    let set = |c: &Clause| {
                        c.ctors
                            .as_ref()
                            .map(|cs| cs.iter().copied().collect::<BTreeSet<usize>>())
                    };
                    c.field == clauses[k].field && set(c) == set(&clauses[k])
                };
                clauses.iter().filter(|c| same(c)).count() > 1
            };
            can.extend(
                (0..clauses.len())
                    .filter(|k| super::super::SEARCH_DROP_OF_DUPLICATE_CLAUSE || !twice(*k))
                    .map(|k| (i, k)),
            );
        }
        let &(i, k) = rng.pick_some(&can)?;
        let r = &mut self.s.rels[i];
        let clauses = if unique {
            &mut r.uniques
        } else {
            &mut r.indexes
        };
        let c = clauses.remove(k);
        let scope = match &c.ctors {
            Some(cs) if r.sum => {
                let mut cs = cs.clone();
                if super::super::SEARCH_DROP_INDEX_REVERSED_SCOPE && !unique {
                    cs.reverse();
                }
                format!("{}.{}", r.name, r.ctor_set(&cs))
            }
            _ => r.name.clone(),
        };
        Some(vec![op(
            kind,
            format!("drop {verb} ({}) in {scope}", c.field),
        )])
    }

    /// Returns the integer field a rekey of `r` keys it by: a header field, or a field of a
    /// record's only constructor.
    fn key_field(r: &Rel) -> Option<String> {
        let only = if r.sum {
            &[][..]
        } else {
            &r.ctors[0].fields[..]
        };
        r.plain_header()
            .iter()
            .chain(only)
            .find(|f| f.ty == Ty::Int && f.action == Action::Unstated)
            .map(|f| f.name.clone())
    }

    pub(super) fn rekey(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let can: Vec<usize> = (0..self.s.rels.len())
            .filter(|j| {
                let r = self.rel(*j);
                let own = r
                    .header
                    .iter()
                    .chain(r.ctors.iter().flat_map(|c| &c.fields))
                    .any(|f| f.ty.target().is_some_and(|(t, _)| t == *j));
                let keyed_by = self.s.rels.iter().any(|s| {
                    s.header[..s.keys]
                        .iter()
                        .any(|f| f.ty.target().is_some_and(|(t, _)| t == *j))
                });
                r.keys == 0
                    && !own
                    && !keyed_by
                    && Self::key_field(r).is_some()
                    && !self.s.extra.fns.iter().any(|f| f.reads == Some(*j))
            })
            .collect();
        let Some(&j) = rng.pick_some(&can) else {
            let r = rng.pick_some(&self.s.rels).filter(|_| broke)?;
            return Some(vec![broken(
                Kind::Rekey,
                format!("rekey relation {} key (zz)", r.name),
            )]);
        };
        let carried: Vec<String> = self
            .refs()
            .iter()
            .filter(|(_, _, f)| f.ty.target().is_some_and(|(t, _)| t == j))
            .map(|(i, c, f)| self.field_path(*i, *c, &f.name))
            .collect();
        let r = self.rel(j);
        let h = Self::key_field(r).expect("a relation rekeyed has an integer field");
        let (name, ty) = (r.name.clone(), r.ty.clone());
        let carry = |fs: &[String]| {
            if fs.is_empty() {
                String::new()
            } else {
                format!("\n  carry {}", fs.join(", "))
            }
        };
        if broke {
            let text = match carried.split_last() {
                Some((_, rest)) => format!("rekey relation {name} key ({h}){}", carry(rest)),
                None => format!("rekey relation {name} key (zz)"),
            };
            return Some(vec![broken(Kind::Rekey, text)]);
        }
        let text = format!("rekey relation {name} key ({h}){}", carry(&carried));
        let target = self.rel(j).name.clone();
        let dropped = Effect::DropField {
            rel: target.clone(),
            ctor: None,
            field: "id".into(),
        };
        let r = &mut self.s.rels[j];
        let f = match r.header.iter().position(|f| f.name == h) {
            Some(at) => r.header.remove(at),
            None => {
                let fs = &mut r.ctors[0].fields;
                let at = fs
                    .iter()
                    .position(|f| f.name == h)
                    .expect("the new key field");
                fs.remove(at)
            }
        };
        r.header.insert(0, f);
        r.keys = 1;
        self.keys.rekeyed(j);
        Some(vec![
            emptying(Kind::Rekey, text, target),
            op(Kind::DropField, format!("drop field {ty}.id")).with(dropped),
        ])
    }

    pub(super) fn default(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let i = rng.below(self.s.rels.len() as u64) as usize;
        let r = self.rel(i);
        let path = |r: &Rel, c: usize, f: &str| {
            if r.sum {
                format!("{}.{}.{f}", r.name, r.ctors[c].name)
            } else {
                format!("{}.{f}", r.name)
            }
        };
        if broke {
            let text = match kind {
                Kind::AddDefault => format!("add default {} = 1", path(r, 0, "zz")),
                _ => format!("drop default {}", path(r, 0, "zz")),
            };
            return Some(vec![broken(kind, text)]);
        }
        if kind == Kind::DropDefault {
            let k = rng.below(r.defaults.len().max(1) as u64) as usize;
            let (c, f, _) = r.defaults.get(k)?.clone();
            if self.retag_target(i, c) {
                return None;
            }
            let text = format!("drop default {}", path(r, c, &f));
            self.s.rels[i].defaults.remove(k);
            return Some(vec![op(kind, text)]);
        }
        let mut can = Vec::new();
        for (c, ctor) in r.ctors.iter().enumerate() {
            for f in &ctor.fields {
                if f.ty.literal() && !r.has_default(c, &f.name) {
                    can.push((c, f.clone()));
                }
            }
        }
        let (c, f) = rng.pick_some(&can)?.clone();
        let v = literal(&f.ty, rng.below(2) == 0);
        let text = format!("add default {} = {v}", path(r, c, &f.name));
        self.s.rels[i].defaults.push((c, f.name, v));
        Some(vec![op(kind, text)])
    }

    pub(super) fn func(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let fns = self.s.extra.fns.clone();
        let pick = |rng: &mut Rng, keep: &dyn Fn(&Func) -> bool| -> Option<usize> {
            let ks: Vec<usize> = (0..fns.len()).filter(|k| keep(&fns[*k])).collect();
            rng.pick_some(&ks).copied()
        };
        let plain = |f: &Func| !f.aggregate;
        match kind {
            Kind::DefineFn => {
                if broke {
                    return Some(vec![broken(
                        kind,
                        "define fn zz (x: I64) -> I64 = x ++ \"a\"".into(),
                    )]);
                }
                let name = self.fresh("f");
                let reads =
                    (rng.below(2) == 0).then(|| rng.below(self.s.rels.len() as u64) as usize);
                let text = self.fn_decl("define", &name, reads, 1);
                self.s.extra.fns.push(Func {
                    name,
                    reads,
                    stored: false,
                    aggregate: false,
                    adds: 1,
                });
                Some(vec![op(kind, text)])
            }
            Kind::RedefineFn => {
                if broke {
                    return Some(vec![broken(kind, "redefine fn zz () -> I64 = 1".into())]);
                }
                let k = pick(rng, &plain)?;
                let f = &fns[k];
                let text = self.fn_decl("redefine", &f.name, f.reads, f.adds + 1);
                self.s.extra.fns[k].adds += 1;
                Some(vec![op(kind, text)])
            }
            Kind::DropFn => {
                if broke {
                    let text = match pick(rng, &|f| f.stored) {
                        Some(k) => format!("drop fn {}", fns[k].name),
                        None => "drop fn zz".into(),
                    };
                    return Some(vec![broken(kind, text)]);
                }
                let k = pick(rng, &|f| plain(f) && !f.stored)?;
                self.s.extra.fns.remove(k);
                Some(vec![op(kind, format!("drop fn {}", fns[k].name))])
            }
            Kind::DefineAggregate => {
                if broke {
                    let k = pick(rng, &|_| true)?;
                    let text = format!("define aggregate {} {AGGREGATE}", fns[k].name);
                    return Some(vec![broken(kind, text)]);
                }
                let name = self.fresh("agg");
                let text = format!("define aggregate {name} {AGGREGATE}");
                self.s.extra.fns.push(Func {
                    name,
                    reads: None,
                    stored: false,
                    aggregate: true,
                    adds: 0,
                });
                Some(vec![op(kind, text)])
            }
            Kind::DropAggregate => {
                if broke {
                    return Some(vec![broken(kind, "drop aggregate zz".into())]);
                }
                let k = pick(rng, &|f| f.aggregate)?;
                self.s.extra.fns.remove(k);
                Some(vec![op(kind, format!("drop aggregate {}", fns[k].name))])
            }
            Kind::Store => {
                if broke {
                    let text = match pick(rng, &|f| f.reads.is_none() && !f.aggregate) {
                        Some(k) => format!("store {}\n  key a", fns[k].name),
                        None => "store zz\n  key a".into(),
                    };
                    return Some(vec![broken(kind, text)]);
                }
                let mut out = Vec::new();
                let k = match pick(rng, &|f| f.reads.is_some() && !f.stored) {
                    Some(k) => k,
                    None => {
                        let name = self.fresh("f");
                        let reads = Some(rng.below(self.s.rels.len() as u64) as usize);
                        out.push(op(Kind::DefineFn, self.fn_decl("define", &name, reads, 1)));
                        self.s.extra.fns.push(Func {
                            name,
                            reads,
                            stored: false,
                            aggregate: false,
                            adds: 1,
                        });
                        self.s.extra.fns.len() - 1
                    }
                };
                let f = &mut self.s.extra.fns[k];
                f.stored = true;
                out.push(op(kind, format!("store {}\n  key a", f.name)));
                Some(out)
            }
            Kind::Unstore => {
                if broke {
                    let k = pick(rng, &|f| !f.stored && !f.aggregate)?;
                    return Some(vec![broken(kind, format!("unstore {}", fns[k].name))]);
                }
                let k = pick(rng, &|f| f.stored)?;
                self.s.extra.fns[k].stored = false;
                Some(vec![op(kind, format!("unstore {}", fns[k].name))])
            }
            _ => unreachable!("a function operator"),
        }
    }

    /// Returns the declaration of function `name` after `verb fn`: of a set of keys of relation
    /// `reads`, else of an integer `x` plus `n`.
    pub(super) fn fn_decl(&self, verb: &str, name: &str, reads: Option<usize>, n: u64) -> String {
        super::fn_decl(&self.s, &format!("{verb} fn"), name, reads, n)
    }

    pub(super) fn constraint(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let mut ints = Vec::new();
        for (i, r) in self.s.rels.iter().enumerate() {
            for (c, ctor) in r.ctors.iter().enumerate() {
                for f in ctor.fields.iter().filter(|f| f.ty == Ty::Int) {
                    ints.push((i, c, f.name.clone()));
                }
            }
        }
        match kind {
            Kind::AddConstraint | Kind::AddCheck => {
                let (i, c, f) = rng.pick_some(&ints)?.clone();
                let f = if broke { "zz".to_owned() } else { f };
                let at = self.rel(i).target(Some(&vec![c]));
                let name = self.fresh(if kind == Kind::AddCheck { "k" } else { "c" });
                if kind == Kind::AddCheck {
                    let text = format!("add check {name} on {at} ({f} >= (-2))");
                    if broke {
                        return Some(vec![broken(kind, text)]);
                    }
                    self.s.rels[i].checks.push(Clause {
                        field: f,
                        ctors: Some(vec![c]),
                        name,
                    });
                    return Some(vec![data(kind, text)]);
                }
                let k = Constraint {
                    rel: i,
                    ctors: vec![c],
                    field: f,
                    name,
                };
                let text = format!("add {}", self.s.constraint_decl(&k));
                if broke {
                    return Some(vec![broken(kind, text)]);
                }
                self.s.constraints.push(k);
                Some(vec![data(kind, text)])
            }
            Kind::DropConstraint => {
                if broke {
                    return Some(vec![broken(kind, "drop constraint zz".into())]);
                }
                let k = rng.below(self.s.constraints.len().max(1) as u64) as usize;
                if k >= self.s.constraints.len() {
                    return None;
                }
                let c = self.s.constraints.remove(k);
                Some(vec![op(kind, format!("drop constraint {}", c.name))])
            }
            _ => {
                if broke {
                    return Some(vec![broken(kind, "drop check zz".into())]);
                }
                let mut can = Vec::new();
                for (i, r) in self.s.rels.iter().enumerate() {
                    can.extend((0..r.checks.len()).map(|k| (i, k)));
                }
                let &(i, k) = rng.pick_some(&can)?;
                let c = self.s.rels[i].checks.remove(k);
                Some(vec![op(kind, format!("drop check {}", c.name))])
            }
        }
    }

    pub(super) fn computed(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let i = rng.below(self.s.rels.len() as u64) as usize;
        let r = self.rel(i).name.clone();
        if kind == Kind::AddComputed {
            if broke {
                return Some(vec![broken(
                    kind,
                    format!("add computed {r}.zz: I64 = \"a\""),
                )]);
            }
            let name = self.field_name(i);
            self.s.extra.computed.push((i, name.clone(), 1));
            return Some(vec![op(kind, format!("add computed {r}.{name}: I64 = 1"))]);
        }
        if broke {
            let text = match kind {
                Kind::RedefineComputed => format!("redefine computed {r}.zz: I64 = 2"),
                _ => format!("drop computed {r}.zz"),
            };
            return Some(vec![broken(kind, text)]);
        }
        let k = rng.below(self.s.extra.computed.len().max(1) as u64) as usize;
        let (j, name, n) = self.s.extra.computed.get(k)?.clone();
        let r = self.rel(j).name.clone();
        if kind == Kind::RedefineComputed {
            self.s.extra.computed[k].2 = n + 1;
            return Some(vec![op(
                kind,
                format!("redefine computed {r}.{name}: I64 = {}", n + 1),
            )]);
        }
        self.s.extra.computed.remove(k);
        Some(vec![op(kind, format!("drop computed {r}.{name}"))])
    }

    pub(super) fn grant(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let privilege = rng.pick(&["read", "insert", "delete"]).to_string();
        let i = rng.below(self.s.rels.len() as u64) as usize;
        let r = self.rel(i).name.clone();
        let held = self
            .s
            .extra
            .grants
            .iter()
            .any(|(p, j)| *p == privilege && *j == i);
        if kind == Kind::Grant {
            if broke {
                return Some(vec![broken(
                    kind,
                    format!("grant {privilege} zz to {ROLE}"),
                )]);
            }
            if !held {
                self.s.extra.grants.push((privilege.clone(), i));
            }
            return Some(vec![op(kind, format!("grant {privilege} {r} to {ROLE}"))]);
        }
        if broke {
            if held {
                return None;
            }
            return Some(vec![broken(
                kind,
                format!("revoke {privilege} {r} from {ROLE}"),
            )]);
        }
        let k = rng.below(self.s.extra.grants.len().max(1) as u64) as usize;
        let (p, j) = self.s.extra.grants.get(k)?.clone();
        self.s.extra.grants.remove(k);
        let r = self.rel(j).name.clone();
        Some(vec![op(kind, format!("revoke {p} {r} from {ROLE}"))])
    }
}
