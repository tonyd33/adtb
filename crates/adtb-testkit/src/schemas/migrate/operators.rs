use std::collections::BTreeSet;

use super::super::{Action, Clause, Constraint, Field, Rel, Stage, Ty, literal};
use super::rows::{Effect, Literal, literal_value};
use super::{AGGREGATE, Func, Kind, Model, Op, ROLE, Reach, broken, data, emptying, fill, op};
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
        let retagging: Vec<(usize, usize)> = fits
            .iter()
            .copied()
            .filter(|&x| st(self, x) == Stage::Enabled)
            .collect();
        let wanted = matches!(
            self.want,
            Some(Reach::DisableRetagTarget | Reach::DeprecateRetagTarget)
        );
        let &(i, c) = match rng.pick_some(&removable) {
            _ if wanted && !retagging.is_empty() => rng.pick(&retagging),
            Some(x) if rng.below(4) != 0 => x,
            _ => rng.pick_some(&fits)?,
        };
        let text = format!("{verb} {}", self.ctor_ref(i, c));
        if broke {
            let enabled = st(self, (i, c)) == Stage::Enabled;
            let reach = match kind {
                Kind::Disable => Reach::DisableRetagTarget,
                _ => Reach::DeprecateRetagTarget,
            };
            let retags = enabled && kind != Kind::Enable;
            return Some(vec![broken(kind, text).reaching(retags.then_some(reach))]);
        }
        let mut out: Vec<Op> = Vec::new();
        if kind == Kind::Deprecate {
            out.extend(self.tag_of(rng, i, &[c], self.removable(i, c)));
        }
        self.s.rels[i].states[c] = match kind {
            Kind::Enable => Stage::Enabled,
            Kind::Disable => Stage::Declared,
            _ => Stage::Deprecated(self.version),
        };
        out.push(if kind == Kind::Disable {
            data(kind, text)
        } else {
            op(kind, text)
        });
        Some(out)
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

    /// Returns the operators adding, over constructor `c` of relation `i` alone, a unique and a
    /// default a split of `c` takes to its products, each added to the model; the unique with a
    /// `Bool` field to route by first when `c` has none.
    pub(super) fn seeded(&mut self, i: usize, c: usize) -> Vec<Op> {
        let r = self.rel(i).clone();
        let fields: Vec<&Field> = r.ctors[c].fields.iter().collect();
        let at = self.ctor_ref(i, c);
        let mut out = Vec::new();
        let own = fields
            .iter()
            .find(|f| f.ty.keyed() && f.ty != Ty::Bool && r.holding(&f.name, &f.ty) == [c]);
        if let Some(f) = own
            && !r
                .uniques
                .iter()
                .any(|k| k.ctors.as_deref() == Some(&[c][..]))
        {
            let flagged = fields
                .iter()
                .any(|f| f.ty == Ty::Bool && f.action == Action::Unstated);
            if !flagged {
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
        if let Some(f) = literal_field {
            let v = literal(&f.ty, false);
            let path = format!("{}.{}.{}", r.name, r.ctors[c].name, f.name);
            out.push(op(Kind::AddDefault, format!("add default {path} = {v}")));
            self.s.rels[i].defaults.push((c, f.name.clone(), v));
        }
        out
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
        if self.version % 4 < 2
            && let Some(ops) = self.shared_field(i, c)
        {
            return Some(ops);
        }
        let name = self.field_name(i);
        let tagged: Vec<usize> = (0..self.s.rels.len())
            .filter(|&j| j != i && self.rel(j).sum && !self.rel(j).enabled().is_empty())
            .collect();
        let (ty, fill, value) = match rng.pick_some(&tagged) {
            Some(&j) if rng.below(3) == 0 => {
                let r = self.rel(j);
                let ctor = r.ctors[*rng.pick(&r.enabled())].name.clone();
                let value = Literal::Tag {
                    rel: r.name.clone(),
                    ctor: ctor.clone(),
                };
                (Ty::TagOf(j), ctor, value)
            }
            _ => {
                let ty = rng.pick(&[Ty::Int, Ty::Text, Ty::Bool, Ty::OptInt]).clone();
                let other = rng.below(2) == 0;
                let fill = literal(&ty, other);
                let value = literal_value(&ty, other);
                (ty, fill, value)
            }
        };
        let text = format!(
            "add field {}: {} fill {fill}",
            self.field_path(i, Some(c), &name),
            self.s.ty_name(&ty),
        );
        let effect = Effect::AddField {
            rel: self.rel(i).name.clone(),
            ctor: Some(self.rel(i).ctors[c].name.clone()),
            field: name.clone(),
            value,
        };
        self.s.rels[i].ctors[c].fields.push(Field::new(&name, ty));
        Some(vec![op(Kind::AddField, text).with(effect)])
    }

    /// Returns, always when `always` and otherwise on half its draws, an `add field` to an
    /// enabled constructor of another relation, a record's when one has one, of a field of type
    /// `Tag` of relation `i`'s sum, filled with the tag of one of constructors `cs` of it.
    pub(super) fn tag_of(
        &mut self,
        rng: &mut Rng,
        i: usize,
        cs: &[usize],
        always: bool,
    ) -> Option<Op> {
        if !self.rel(i).sum || (!always && rng.below(2) == 0) {
            return None;
        }
        let holders: Vec<((usize, usize), bool)> = (0..self.s.rels.len())
            .filter(|&h| h != i)
            .flat_map(|h| self.rel(h).enabled().into_iter().map(move |d| (h, d)))
            .filter(|&(h, d)| !self.retag_target(h, d))
            .map(|(h, d)| ((h, d), !self.rel(h).sum))
            .collect();
        let holders = Self::steered(holders);
        let &(h, d) = rng.pick_some(&holders)?;
        let ctor = self.rel(i).ctors[*rng.pick(cs)].name.clone();
        let name = self.field_name(h);
        let ty = Ty::TagOf(i);
        let text = format!(
            "add field {}: {} fill {ctor}",
            self.field_path(h, Some(d), &name),
            self.s.ty_name(&ty),
        );
        let effect = Effect::AddField {
            rel: self.rel(h).name.clone(),
            ctor: Some(self.rel(h).ctors[d].name.clone()),
            field: name.clone(),
            value: Literal::Tag {
                rel: self.rel(i).name.clone(),
                ctor,
            },
        };
        self.s.rels[h].ctors[d].fields.push(Field::new(&name, ty));
        Some(op(Kind::AddField, text).with(effect))
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
            .filter(|(i, c, f)| {
                !self.names_beside_defaults(*i, Some(*c), &f.name)
                    && !self.retag_target(*i, *c)
                    && f.action != Action::Retag
                    && !self.s.extra.computed.iter().any(|(r, _, _)| r == i)
            })
            .collect();
        let peers = |(i, c, f): &(usize, usize, Field)| self.default_peers(*i, *c, &f.name);
        let steered: Vec<(usize, usize, Field)> = match self.want {
            Some(Reach::DroppedBesideDefault) => can
                .iter()
                .filter(|x| !peers(x).is_empty())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let from = if steered.is_empty() { &can } else { &steered };
        let picked = rng.pick_some(from)?.clone();
        let beside = peers(&picked);
        let (i, c, f) = picked;
        let text = format!("drop field {}", self.field_path(i, Some(c), &f.name));
        let r = self.rel(i);
        let renaming = beside
            .iter()
            .map(|&d| (r.name.clone(), r.ctors[d].name.clone(), f.name.clone()));
        self.s.extra.renaming.extend(renaming.collect::<Vec<_>>());
        let effect = Effect::DropField {
            rel: self.rel(i).name.clone(),
            ctor: Some(self.rel(i).ctors[c].name.clone()),
            field: f.name.clone(),
        };
        let r = &mut self.s.rels[i];
        r.ctors[c].fields.retain(|g| g.name != f.name);
        r.defaults.retain(|(d, g, _)| *d != c || *g != f.name);
        let reach = (!beside.is_empty()).then_some(Reach::DroppedBesideDefault);
        Some(vec![Op {
            fill: f.ty.literal().then(|| literal(&f.ty, false)),
            ..op(Kind::DropField, text).with(effect).reaching(reach)
        }])
    }

    /// Returns the other constructors of relation `i` whose field `f` has a default of the value
    /// of the default of field `f` of constructor `c`, none when that field has no default.
    fn default_peers(&self, i: usize, c: usize, f: &str) -> Vec<usize> {
        let r = self.rel(i);
        let Some((_, _, v)) = r.defaults.iter().find(|(d, g, _)| *d == c && g == f) else {
            return Vec::new();
        };
        r.defaults
            .iter()
            .filter(|(d, g, w)| *d != c && g == f && w == v)
            .map(|(d, _, _)| *d)
            .collect()
    }

    /// Returns a broken `rename field` of one of `fields` that a unique or index names together
    /// with the field of its name of another constructor.
    fn shared_rename(&mut self, fields: &[(usize, usize, Field)]) -> Option<Vec<Op>> {
        let (j, d, g) = fields
            .iter()
            .find(|(j, d, g)| {
                let r = self.rel(*j);
                r.uniques.iter().chain(&r.indexes).any(|k| {
                    k.field == g.name
                        && k.ctors
                            .as_ref()
                            .is_none_or(|cs| cs.len() > 1 && cs.contains(d))
                }) && r.ctors.len() > 1
            })?
            .clone();
        let path = self.field_path(j, Some(d), &g.name);
        let to = self.field_name(j);
        let text = format!("rename field {path} to {to}");
        Some(vec![
            broken(Kind::RenameField, text).reaching(Some(Reach::SharedElementRenamed)),
        ])
    }

    /// Returns an `add field` to constructor `c` of sum relation `i` of an integer or text field
    /// of the name and type of one another constructor holds, then an index over every
    /// constructor holding it, adding both to the model.
    fn shared_field(&mut self, i: usize, c: usize) -> Option<Vec<Op>> {
        let r = self.rel(i);
        if !r.sum {
            return None;
        }
        let f = r
            .ctors
            .iter()
            .flat_map(|k| &k.fields)
            .find(|f| {
                matches!(f.ty, Ty::Int | Ty::Text)
                    && r.ctors[c].field(&f.name).is_none()
                    && r.header.iter().all(|h| h.name != f.name)
            })?
            .clone();
        let text = format!(
            "add field {}: {} fill {}",
            self.field_path(i, Some(c), &f.name),
            self.s.ty_name(&f.ty),
            literal(&f.ty, false),
        );
        let effect = Effect::AddField {
            rel: r.name.clone(),
            ctor: Some(r.ctors[c].name.clone()),
            field: f.name.clone(),
            value: literal_value(&f.ty, false),
        };
        self.s.rels[i].ctors[c]
            .fields
            .push(Field::new(&f.name, f.ty.clone()));
        let r = self.rel(i);
        let holding = r.holding(&f.name, &f.ty);
        let index = format!("add index ({}) in {}", f.name, r.target(Some(&holding)));
        self.s.rels[i].indexes.push(Clause {
            field: f.name,
            ctors: Some(holding),
            name: String::new(),
        });
        Some(vec![
            op(Kind::AddField, text).with(effect),
            op(Kind::AddIndex, index),
        ])
    }

    pub(super) fn rename_field(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let fields = self.own_fields();
        let at = |(i, c, f): &(usize, usize, Field)| {
            let r = self.rel(*i);
            (r.name.clone(), r.ctors[*c].name.clone(), f.name.clone())
        };
        let latest = self
            .s
            .extra
            .renaming
            .iter()
            .rev()
            .find_map(|x| fields.iter().find(|g| at(g) == *x))
            .filter(|_| !broke);
        let steered: Vec<(usize, usize, Field)> = latest.cloned().into_iter().collect();
        let from = if steered.is_empty() {
            &fields
        } else {
            &steered
        };
        let (i, c, f) = rng.pick_some(from)?.clone();
        let reach = (!steered.is_empty()).then_some(Reach::RenamedBesideDropped);
        let path = self.field_path(i, Some(c), &f.name);
        if broke
            && (self.want == Some(Reach::SharedElementRenamed) || self.version % 4 < 2)
            && let Some(ops) = self.shared_rename(&fields)
        {
            return Some(ops);
        }

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
        let bound = super::super::BOUND;
        let free = r.ctors.iter().all(|k| k.field(bound).is_none())
            && r.header.iter().all(|h| h.name != bound);
        let to = if free && rng.below(2) == 0 {
            bound.to_owned()
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
            op(Kind::RenameField, format!("rename field {path} to {to}"))
                .with(effect)
                .reaching(reach),
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
                if !to.is_empty() && !self.clause_names_field(i, None, &f.name) {
                    can.push((i, f.clone(), to));
                }
            }
        }
        let wanted = self.want == Some(Reach::MovedDownDefault);
        let acting = |f: &Field| !matches!(f.action, Action::Unstated | Action::Restrict);
        if self.want == Some(Reach::MovedDownAction) && can.iter().any(|(_, f, _)| acting(f)) {
            can.retain(|(_, f, _)| acting(f));
        }
        let several: Vec<(usize, Field, Vec<usize>)> = can
            .iter()
            .filter(|(_, f, to)| to.len() > 1 && f.ty.literal())
            .cloned()
            .collect();
        let (i, f, all) = match wanted && !several.is_empty() {
            true => rng.pick(&several).clone(),
            false => rng.pick_some(&can)?.clone(),
        };
        let mut to: Vec<usize> = if wanted || self.version.is_multiple_of(2) {
            all.clone()
        } else {
            all.iter().copied().filter(|_| rng.below(2) == 0).collect()
        };
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
        let acted = acting(&f);
        let defaulted = (wanted && to.len() > 1 && f.ty.literal()).then(|| literal(&f.ty, false));
        let mut out = Vec::new();
        let r = &mut self.s.rels[i];
        if let Some(v) = &defaulted {
            out.push(op(
                Kind::AddDefault,
                format!("add default {}.{} = {v}", r.name, f.name),
            ));
        }
        r.header.retain(|g| g.name != f.name);
        for &c in &to {
            r.ctors[c].fields.push(f.clone());
            if let Some(v) = &defaulted {
                r.defaults.push((c, f.name.clone(), v.clone()));
            }
        }
        let reach = if acted {
            Some(Reach::MovedDownAction)
        } else {
            defaulted.map(|_| Reach::MovedDownDefault)
        };
        out.push(op(Kind::MoveFieldDown, text).with(effect).reaching(reach));
        Some(out)
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
        let held = if kind == Kind::AddUnique {
            &r.uniques
        } else {
            &r.indexes
        };
        if broke {
            let text = match held.is_empty() || rng.below(2) == 0 {
                true => format!("add {verb} (zz) in {}", r.name),
                false => {
                    let c = rng.pick(held);
                    format!("add {verb} ({}) in {}", c.field, r.target(c.ctors.as_ref()))
                }
            };
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
                let scope = (r.sum).then_some(holding);
                if !fields.iter().any(|(n, s)| *n == f.name && *s == scope) {
                    fields.push((f.name.clone(), scope));
                }
            }
        }
        let same = |a: &Option<Vec<usize>>, b: &Option<Vec<usize>>| {
            let set = |x: &Option<Vec<usize>>| {
                x.as_ref()
                    .map(|cs| cs.iter().copied().collect::<BTreeSet<usize>>())
            };
            set(a) == set(b)
        };
        let clause = |(field, ctors): &(String, Option<Vec<usize>>)| Clause {
            field: field.clone(),
            ctors: ctors.clone(),
            name: String::new(),
        };
        if kind == Kind::AddUnique {
            fields.retain(|f| !r.holds_unique(&clause(f)));
        } else {
            fields.retain(|(f, cs)| !held.iter().any(|c| c.field == *f && same(&c.ctors, cs)));
        }
        let (field, ctors) = rng.pick_some(&fields)?.clone();
        let clause = clause(&(field.clone(), ctors.clone()));
        let text = format!("add {verb} ({field}) in {}", r.target(ctors.as_ref()));
        let r = &mut self.s.rels[i];
        let added = if kind == Kind::AddUnique {
            r.uniques.push(clause);
            data(kind, text)
        } else {
            r.indexes.push(clause);
            op(kind, text)
        };
        Some(vec![added])
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
            can.extend((0..clauses.len()).map(|k| (i, k)));
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
        if !broke
            && kind == Kind::AddDefault
            && self.want == Some(Reach::DefaultOverSeveral)
            && let Some(ops) = self.default_over_several(rng)
        {
            return Some(ops);
        }
        if !broke
            && kind == Kind::DropDefault
            && self.want == Some(Reach::DroppedBesideField)
            && let Some(ops) = self.dropped_by_name(rng)
        {
            return Some(ops);
        }
        let before_ctor = self.want == Some(Reach::DefaultBeforeCtor);
        let sums = self.sums();
        let i = match before_ctor && !sums.is_empty() {
            true => *rng.pick(&sums),
            false => rng.below(self.s.rels.len() as u64) as usize,
        };
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
            let filled = (r.name.clone(), r.ctors[c].name.clone(), f);
            self.s.extra.renaming.push(filled);
            self.s.rels[i].defaults.remove(k);
            return Some(vec![op(kind, text)]);
        }
        let mut can = Vec::new();
        for (c, ctor) in r.ctors.iter().enumerate() {
            for f in &ctor.fields {
                if f.ty.literal() && !r.has_default(c, &f.name) && !self.retag_target(i, c) {
                    can.push((c, f.clone()));
                }
            }
        }
        let ints: Vec<(usize, Field)> = can
            .iter()
            .filter(|(_, f)| f.ty == Ty::Int)
            .cloned()
            .collect();
        let (c, f) = match before_ctor && !ints.is_empty() {
            true => rng.pick(&ints).clone(),
            false => rng.pick_some(&can)?.clone(),
        };
        let v = literal(&f.ty, rng.below(2) == 0);
        if r.sum
            && (before_ctor || self.version.is_multiple_of(2))
            && let Some(ops) = self.default_before_ctor(i, &f, &v)
        {
            return Some(ops);
        }
        let r = self.rel(i);
        let text = format!("add default {} = {v}", path(r, c, &f.name));
        self.s.rels[i].defaults.push((c, f.name, v));
        Some(vec![op(kind, text)])
    }

    /// Returns the constructors of sum relation `i` holding a field `f` of type `ty`, when each
    /// has no default for it, is not the target of a retag action and holds it with that type;
    /// else none.
    fn undefaulted(&self, i: usize, f: &str, ty: &Ty) -> Vec<usize> {
        let r = self.rel(i);
        let holding: Vec<usize> = (0..r.ctors.len())
            .filter(|&k| r.ctors[k].field(f).is_some())
            .collect();
        let free = holding.iter().all(|&k| {
            !r.has_default(k, f)
                && !self.retag_target(i, k)
                && r.ctors[k].field(f).is_some_and(|g| g.ty == *ty)
        });
        if free && r.header.iter().all(|h| h.name != f) {
            holding
        } else {
            Vec::new()
        }
    }

    /// Returns an `add default` naming no constructor of integer field `f` of sum relation `i`,
    /// then an `add constructor` of a field of that name, which the default does not fill,
    /// adding them to the model; `None` unless each constructor holding `f` may take it.
    fn default_before_ctor(&mut self, i: usize, f: &Field, v: &str) -> Option<Vec<Op>> {
        if f.ty != Ty::Int {
            return None;
        }
        let holding = self.undefaulted(i, &f.name, &f.ty);
        if holding.is_empty() {
            return None;
        }
        let text = format!("add default {}.{} = {v}", self.rel(i).name, f.name);
        let ty = self.rel(i).ty.clone();
        let name = self.ctor_name(i);
        let added = format!("add constructor {ty}.{name} {{ {}: I64 }}", f.name);
        let r = &mut self.s.rels[i];
        for k in holding {
            r.defaults.push((k, f.name.clone(), v.to_owned()));
        }
        r.ctors.push(super::super::Ctor {
            name,
            fields: vec![f.clone()],
        });
        r.states.push(Stage::Declared);
        Some(vec![
            op(Kind::AddDefault, text),
            op(Kind::AddConstructor, added).reaching(Some(Reach::DefaultBeforeCtor)),
        ])
    }

    /// Returns an `add default` naming no constructor of an integer or text field that two or
    /// more constructors of a sum relation hold, adding it to the model.
    fn default_over_several(&mut self, rng: &mut Rng) -> Option<Vec<Op>> {
        let mut can = Vec::new();
        for i in self.sums() {
            let r = self.rel(i);
            let mut seen = BTreeSet::new();
            let fields = r.ctors.iter().flat_map(|c| &c.fields);
            for f in fields.filter(|f| seen.insert(f.name.clone())) {
                let holding = self.undefaulted(i, &f.name, &f.ty);
                if matches!(f.ty, Ty::Int | Ty::Text) && holding.len() > 1 {
                    can.push((i, f.clone(), holding));
                }
            }
        }
        let (i, f, holding) = rng.pick_some(&can)?.clone();
        let v = literal(&f.ty, false);
        let r = &mut self.s.rels[i];
        let text = format!("add default {}.{} = {v}", r.name, f.name);
        for k in holding {
            r.defaults.push((k, f.name.clone(), v.clone()));
        }
        Some(vec![
            op(Kind::AddDefault, text).reaching(Some(Reach::DefaultOverSeveral)),
        ])
    }

    /// Returns a `drop default` naming no constructor of a field name of a sum relation that a
    /// constructor holds with a default and another without one, dropping the default of each
    /// field of that name from the model.
    fn dropped_by_name(&mut self, rng: &mut Rng) -> Option<Vec<Op>> {
        let mut can = Vec::new();
        for i in self.sums() {
            let r = self.rel(i);
            for (c, g, _) in &r.defaults {
                let bare = (0..r.ctors.len())
                    .any(|k| r.ctors[k].field(g).is_some() && !r.has_default(k, g));
                let held: Vec<usize> = r
                    .defaults
                    .iter()
                    .filter(|(_, h, _)| h == g)
                    .map(|(d, _, _)| *d)
                    .collect();
                if bare
                    && r.ctors[*c].field(g).is_some()
                    && !held.iter().any(|&d| self.retag_target(i, d))
                {
                    can.push((i, g.clone(), held));
                }
            }
        }
        let (i, f, held) = rng.pick_some(&can)?.clone();
        let r = self.rel(i);
        let text = format!("drop default {}.{f}", r.name);
        let filled: Vec<(String, String, String)> = held
            .iter()
            .map(|&d| (r.name.clone(), r.ctors[d].name.clone(), f.clone()))
            .collect();
        self.s.extra.renaming.extend(filled);
        self.s.rels[i].defaults.retain(|(_, g, _)| *g != f);
        Some(vec![
            op(Kind::DropDefault, text).reaching(Some(Reach::DroppedBesideField)),
        ])
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
