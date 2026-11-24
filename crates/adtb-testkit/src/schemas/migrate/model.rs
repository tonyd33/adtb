use super::super::draw::Draw;
use super::super::{Action, Clause, Ctor, Field, Rel, Scope, Stage, Ty};
use super::{Effect, Kind, Model, Op, Remap, broken, data, emptying, op, remapped};
use crate::Rng;

impl Model {
    /// Returns a name of `prefix` and a number past those of every generated schema's names.
    pub(super) fn fresh(&mut self, prefix: &str) -> String {
        self.s.extra.next += 1;
        format!("{prefix}{}", 100 + self.s.extra.next)
    }

    pub(super) fn rel(&self, i: usize) -> &Rel {
        &self.s.rels[i]
    }

    /// Returns the relations that are sums.
    pub(super) fn sums(&self) -> Vec<usize> {
        (0..self.s.rels.len())
            .filter(|i| self.s.rels[*i].sum)
            .collect()
    }

    /// Returns every reference field of the schema: its relation, constructor (`None` in the
    /// header) and field.
    pub(super) fn refs(&self) -> Vec<(usize, Option<usize>, &Field)> {
        let mut out = Vec::new();
        for (i, r) in self.s.rels.iter().enumerate() {
            out.extend(r.header.iter().map(|f| (i, None, f)));
            for (c, ctor) in r.ctors.iter().enumerate() {
                out.extend(ctor.fields.iter().map(|f| (i, Some(c), f)));
            }
        }
        out.retain(|(_, _, f)| f.ty.target().is_some());
        out
    }

    /// Returns whether a field of a relation other than `j` references `j`.
    pub(super) fn referenced(&self, j: usize) -> bool {
        self.refs()
            .iter()
            .any(|(i, _, f)| *i != j && f.ty.target().is_some_and(|(t, _)| t == j))
    }

    /// Returns whether a field's type is `Tag T` of the sum of relation `j`.
    pub(super) fn tagged(&self, j: usize) -> bool {
        self.s.rels.iter().any(|r| {
            r.header
                .iter()
                .chain(r.ctors.iter().flat_map(|c| &c.fields))
                .any(|f| f.ty == Ty::TagOf(j))
        })
    }

    /// Returns whether a reference type names a constructor set of relation `i` holding `c`
    /// other than the whole relation.
    pub(super) fn selected(&self, i: usize, c: usize) -> bool {
        let set = self.rel(i).set.clone().unwrap_or_default();
        self.refs().iter().any(|(_, _, f)| match f.ty.target() {
            Some((t, Scope::Ctors(cs))) => t == i && cs.contains(&c),
            Some((t, Scope::Named)) => t == i && set.contains(&c),
            _ => false,
        })
    }

    /// Returns whether a reference type, unique, index or check names relation `i`'s set.
    pub(super) fn set_named(&self, i: usize) -> bool {
        let r = self.rel(i);
        let named = self
            .refs()
            .iter()
            .any(|(_, _, f)| matches!(f.ty.target(), Some((t, Scope::Named)) if t == i));
        named
            || r.checks
                .iter()
                .any(|k| k.ctors.is_some() && k.ctors == r.set)
    }

    /// Returns whether an operator of `kind` may replace constructors `cs` of relation `i` by
    /// `products` constructors under the clauses, constraints and defaults naming them.
    pub(super) fn replaces_under(
        &self,
        i: usize,
        cs: &[usize],
        products: usize,
        kind: Kind,
    ) -> bool {
        !self.blocked(i, cs, products, kind)
    }

    /// Returns whether a clause, constraint or default naming constructors `cs` of relation `i`
    /// refuses an operator of `kind` replacing them by `products`: one with no exact image.
    fn blocked(&self, i: usize, cs: &[usize], products: usize, kind: Kind) -> bool {
        let r = self.rel(i);
        let all: Vec<usize> = (0..r.ctors.len()).collect();
        let scope = |k: &Clause| k.ctors.clone().unwrap_or_else(|| all.clone());
        let meets = |s: &[usize]| s.iter().any(|c| cs.contains(c));
        let within = |s: &[usize]| s.iter().all(|c| cs.contains(c));
        let holds = |s: &[usize]| cs.iter().all(|c| s.contains(c));
        let joining = cs.len() > 1;
        let header = |f: &str| r.plain_header().iter().any(|h| h.name == f);
        let no_image = |s: &[usize]| joining && meets(s) && !holds(s) && !within(s);
        let some = |s: &[usize]| joining && meets(s) && within(s) && !holds(s);
        let clause = |k: &Clause| {
            let s = scope(k);
            let nest = kind == Kind::Nest
                && meets(&s)
                && if header(&k.field) {
                    some(&s)
                } else {
                    s.len() > 1
                };
            no_image(&s) || nest || (kind == Kind::Merge && some(&s))
        };
        let check = |k: &Clause| {
            let s = scope(k);
            match kind {
                Kind::Merge => no_image(&s),
                Kind::Nest => meets(&s) && !within(&s),
                _ => joining && meets(&s) && !holds(&s),
            }
        };
        let constraint = |k: &super::super::Constraint| {
            k.rel == i && joining && meets(&k.ctors) && !holds(&k.ctors)
        };
        let flag = (kind == Kind::Split && products > 1)
            .then(|| {
                r.ctors[cs[0]]
                    .fields
                    .iter()
                    .find(|f| f.ty == Ty::Bool && f.action == Action::Unstated)
            })
            .flatten()
            .map(|f| f.name.as_str());
        let dropped = |k: &Clause| Some(k.field.as_str()) == flag && meets(&scope(k));
        let folds = matches!(kind, Kind::Merge | Kind::Nest | Kind::Unnest);
        let default = r
            .defaults
            .iter()
            .any(|(d, f, _)| cs.contains(d) && (folds || Some(f.as_str()) == flag));
        r.uniques.iter().any(|k| clause(k) || dropped(k))
            || r.indexes.iter().any(|k| clause(k) || dropped(k))
            || r.checks.iter().any(|k| check(k) || dropped(k))
            || self.s.constraints.iter().any(constraint)
            || default
    }

    /// Returns the candidates of `can` marked steered, or all of `can` when none is.
    pub(super) fn steered<T>(can: Vec<(T, bool)>) -> Vec<T> {
        let switched = can.iter().any(|(_, s)| *s);
        can.into_iter()
            .filter(|(_, s)| *s || !switched)
            .map(|(x, _)| x)
            .collect()
    }

    /// Returns whether a clause, constraint or default of relation `i` names field `f` of
    /// constructor `c`, or of the header when `c` is `None`.
    pub(super) fn clause_names_field(&self, i: usize, c: Option<usize>, f: &str) -> bool {
        let r = self.rel(i);
        let covers = |k: &Clause| match (&k.ctors, c) {
            (None, _) | (_, None) => true,
            (Some(cs), Some(c)) => cs.contains(&c),
        };
        r.uniques
            .iter()
            .chain(&r.indexes)
            .chain(&r.checks)
            .any(|k| k.field == f && covers(k))
            || r.defaults
                .iter()
                .any(|(d, g, _)| g == f && c.is_none_or(|c| c == *d))
            || self
                .s
                .constraints
                .iter()
                .any(|k| k.rel == i && k.field == f && c.is_none_or(|c| k.ctors.contains(&c)))
    }

    /// Returns whether constructor `c` of relation `i` is the constructor its delete actions
    /// retag into.
    pub(super) fn retag_target(&self, i: usize, c: usize) -> bool {
        let r = self.rel(i);
        r.retag == Some(c)
            && r.ctors
                .iter()
                .flat_map(|k| &k.fields)
                .any(|f| f.action == Action::Retag)
    }

    /// Returns `r.C` naming constructor `c` of relation `i`.
    pub(super) fn ctor_ref(&self, i: usize, c: usize) -> String {
        format!("{}.{}", self.rel(i).name, self.rel(i).ctors[c].name)
    }

    /// Returns `T.C` naming constructor `c` of relation `i`'s type.
    pub(super) fn ctor_path(&self, i: usize, c: usize) -> String {
        format!("{}.{}", self.rel(i).ty, self.rel(i).ctors[c].name)
    }

    /// Returns the path of field `f` of constructor `c` of relation `i`, `T.f` in the header or
    /// in a record.
    pub(super) fn field_path(&self, i: usize, c: Option<usize>, f: &str) -> String {
        let r = self.rel(i);
        match c {
            Some(c) if r.sum => format!("{}.{}.{f}", r.ty, r.ctors[c].name),
            _ => format!("{}.{f}", r.ty),
        }
    }

    /// Returns a name no constructor or set of relation `i` holds.
    pub(super) fn ctor_name(&mut self, i: usize) -> String {
        loop {
            let n = self.fresh("E");
            let r = self.rel(i);
            if r.ctors.iter().all(|c| c.name != n) && r.set_name != n {
                return n;
            }
        }
    }

    /// Returns a field name neither the header of relation `i` nor any of its constructors
    /// holds.
    pub(super) fn field_name(&mut self, i: usize) -> String {
        loop {
            let n = self.fresh("g");
            let r = self.rel(i);
            let taken = r.header.iter().any(|f| f.name == n)
                || r.ctors.iter().any(|c| c.field(&n).is_some());
            if !taken {
                return n;
            }
        }
    }

    /// Replace the constructors `olds` of relation `i` by `products` at the place of the first
    /// of them, in the states `states`. A scope holding every one of `olds` holds every product;
    /// one holding some of them is narrowed to the rest, and one left empty is gone.
    pub(super) fn replace(
        &mut self,
        i: usize,
        olds: &[usize],
        products: Vec<Ctor>,
        states: Vec<Stage>,
    ) {
        let n = self.rel(i).ctors.len();
        let first = olds.iter().copied().min();
        let mut map: Remap = vec![Vec::new(); n];
        let mut ctors = Vec::new();
        let mut stages = Vec::new();
        let r = &self.s.rels[i];
        for c in 0..n {
            if Some(c) == first {
                for (p, s) in products.iter().zip(&states) {
                    for o in olds {
                        map[*o].push(ctors.len());
                    }
                    ctors.push(p.clone());
                    stages.push(*s);
                }
            }
            if olds.contains(&c) {
                continue;
            }
            map[c].push(ctors.len());
            ctors.push(r.ctors[c].clone());
            stages.push(r.states[c]);
        }
        let whole = |cs: &[usize]| cs.len() == n;
        let r = &mut self.s.rels[i];
        r.ctors = ctors;
        r.states = stages;
        for rel in &mut self.s.rels {
            for f in rel
                .header
                .iter_mut()
                .chain(rel.ctors.iter_mut().flat_map(|c| &mut c.fields))
            {
                if let Ty::Ref(t, Scope::Ctors(cs)) | Ty::OptRef(t, Scope::Ctors(cs)) = &mut f.ty
                    && *t == i
                {
                    *cs = remapped(&map, cs);
                }
            }
        }
        let r = &mut self.s.rels[i];
        if let Some(set) = &r.set {
            let set = remapped(&map, set);
            r.set = (!set.is_empty()).then_some(set);
        }
        r.retag = r.retag.and_then(|d| map[d].first().copied());
        for clauses in [&mut r.uniques, &mut r.indexes, &mut r.checks] {
            clauses.retain_mut(|k| match &k.ctors {
                Some(cs) if whole(cs) => true,
                Some(cs) => {
                    let cs = remapped(&map, cs);
                    let kept = !cs.is_empty();
                    k.ctors = Some(cs);
                    kept
                }
                None => true,
            });
        }
        let defaults = std::mem::take(&mut r.defaults);
        r.defaults = defaults
            .into_iter()
            .flat_map(|(c, f, v)| {
                let held: Vec<usize> = map[c]
                    .iter()
                    .copied()
                    .filter(|&d| r.ctors[d].field(&f).is_some())
                    .collect();
                held.into_iter().map(move |d| (d, f.clone(), v.clone()))
            })
            .collect();
        self.s.constraints.retain_mut(|k| {
            if k.rel != i {
                return true;
            }
            k.ctors = remapped(&map, &k.ctors);
            !k.ctors.is_empty()
        });
    }

    /// Remove relation `j`, lowering every index past it.
    pub(super) fn remove_rel(&mut self, j: usize) {
        self.s.rels.remove(j);
        self.keys.remove(j);
        let lower = |x: &mut usize| *x -= usize::from(*x > j);
        for r in &mut self.s.rels {
            for f in r
                .header
                .iter_mut()
                .chain(r.ctors.iter_mut().flat_map(|c| &mut c.fields))
            {
                if let Ty::Ref(t, _) | Ty::OptRef(t, _) | Ty::TagOf(t) = &mut f.ty {
                    lower(t);
                }
            }
        }
        self.s.constraints.retain(|k| k.rel != j);
        for k in &mut self.s.constraints {
            lower(&mut k.rel);
        }
        let x = &mut self.s.extra;
        x.fns.retain(|f| f.reads != Some(j));
        for f in &mut x.fns {
            if let Some(r) = &mut f.reads {
                lower(r);
            }
        }
        x.computed.retain(|(r, _, _)| *r != j);
        x.grants.retain(|(_, r)| *r != j);
        for (r, _, _) in &mut x.computed {
            lower(r);
        }
        for (_, r) in &mut x.grants {
            lower(r);
        }
    }

    /// Returns whether a field, function or merge or nest uses the added type `t`.
    pub(super) fn type_used(&self, t: &str) -> bool {
        self.s.rels.iter().any(|r| {
            r.header
                .iter()
                .chain(r.ctors.iter().flat_map(|c| &c.fields))
                .any(|f| matches!(&f.ty, Ty::Tag(n, _) if n == t))
        })
    }

    /// Draw one operator of `kind`, broken when `broken`, with the operators it needs before
    /// it. Returns `None` when the model holds nothing it can apply to.
    pub(super) fn draw(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        match kind {
            Kind::AddModule
            | Kind::DropModule
            | Kind::AddType
            | Kind::DropType
            | Kind::RenameType => self.object(rng, kind, broke),
            Kind::AddRelation | Kind::DropRelation | Kind::RenameRelation => {
                self.relation(rng, kind, broke)
            }
            Kind::AddSet | Kind::DropSet | Kind::RenameSet => self.set(rng, kind, broke),
            Kind::AddConstructor | Kind::RemoveConstructor | Kind::RenameConstructor => {
                self.constructor(rng, kind, broke)
            }
            Kind::Enable | Kind::Disable | Kind::Deprecate => self.stage(rng, kind, broke),
            Kind::Split => self.split(rng, broke),
            Kind::Merge | Kind::Nest => self.merge(rng, kind, broke),
            Kind::Unnest => self.unnest(rng, broke),
            Kind::AddField => self.add_field(rng, broke),
            Kind::DropField => self.drop_field(rng, broke),
            Kind::RenameField => self.rename_field(rng, broke),
            Kind::MoveFieldUp => self.move_up(rng, broke),
            Kind::MoveFieldDown => self.move_down(rng, broke),
            Kind::AlterField => self.alter_field(rng, broke),
            Kind::AddUnique | Kind::AddIndex => self.add_clause(rng, kind, broke),
            Kind::DropUnique | Kind::DropIndex => self.drop_clause(rng, kind, broke),
            Kind::Rekey => self.rekey(rng, broke),
            Kind::AddDefault | Kind::DropDefault => self.default(rng, kind, broke),
            Kind::DefineFn
            | Kind::RedefineFn
            | Kind::DropFn
            | Kind::DefineAggregate
            | Kind::DropAggregate
            | Kind::Store
            | Kind::Unstore => self.func(rng, kind, broke),
            Kind::AddConstraint | Kind::DropConstraint | Kind::AddCheck | Kind::DropCheck => {
                self.constraint(rng, kind, broke)
            }
            Kind::AddComputed | Kind::RedefineComputed | Kind::DropComputed => {
                self.computed(rng, kind, broke)
            }
            Kind::Grant | Kind::Revoke => self.grant(rng, kind, broke),
            Kind::Script => {
                let i = rng.below(self.s.rels.len() as u64) as usize;
                if broke {
                    return Some(vec![broken(kind, "script\n  delete x in zz".into())]);
                }
                let r = self.rel(i);
                let text = format!("script\n  delete x in {} where key x == {}", r.name, {
                    let d = Draw::resume(&self.s, self.keys.clone());
                    d.some_key(rng, i)
                });
                Some(vec![data(kind, text).with(Effect::Unknown(
                    "the model does not run a script".into(),
                ))])
            }
        }
    }

    /// Draw a module or type operator of `kind`, as [`Model::draw`] does.
    fn object(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        match kind {
            Kind::AddModule => Some(vec![if broke {
                broken(kind, "add module app".into())
            } else {
                let m = self.fresh("m");
                self.s.extra.modules.push(m.clone());
                op(kind, format!("add module {m}"))
            }]),
            Kind::DropModule => {
                if broke {
                    return Some(vec![broken(kind, "drop module app".into())]);
                }
                let mut out = Vec::new();
                if self.s.extra.modules.is_empty() {
                    out.extend(self.draw(rng, Kind::AddModule, false)?);
                }
                let k = rng.below(self.s.extra.modules.len() as u64) as usize;
                let m = self.s.extra.modules.remove(k);
                out.push(op(kind, format!("drop module {m}")));
                Some(out)
            }
            Kind::AddType => Some(vec![if broke {
                broken(kind, "add type V0 = Z".into())
            } else {
                let t = self.fresh("X");
                let n = 100 + self.s.extra.next;
                let cs = vec![format!("P{n}"), format!("Q{n}")];
                let text = format!("add type {t} = {}", cs.join(" | "));
                self.s.extra.types.push((t, cs));
                op(kind, text)
            }]),
            Kind::DropType => {
                if broke {
                    return Some(vec![broken(kind, "drop type V0".into())]);
                }
                let mut out = Vec::new();
                let unused = |m: &Model| -> Vec<usize> {
                    (0..m.s.extra.types.len())
                        .filter(|k| !m.type_used(&m.s.extra.types[*k].0))
                        .collect()
                };
                if unused(self).is_empty() {
                    out.extend(self.draw(rng, Kind::AddType, false)?);
                }
                let k = *rng.pick(&unused(self));
                let (t, _) = self.s.extra.types.remove(k);
                out.push(op(kind, format!("drop type {t}")));
                Some(out)
            }
            Kind::RenameType => {
                let i = rng.below(self.s.rels.len() as u64) as usize;
                let from = self.rel(i).ty.clone();
                if broke {
                    return Some(vec![broken(kind, format!("rename type {from} to V0"))]);
                }
                let to = self.fresh("U");
                let r = &mut self.s.rels[i];
                r.ty.clone_from(&to);
                let text = format!("rename type {from} to {to}");
                if r.sum {
                    return Some(vec![op(kind, text)]);
                }
                r.ctors[0].name.clone_from(&to);
                let effect = Effect::RenameCtor {
                    rel: r.name.clone(),
                    from,
                    to,
                };
                Some(vec![op(kind, text).with(effect)])
            }
            other => unreachable!("{other:?} is no module or type operator"),
        }
    }

    /// Draw a relation operator of `kind`, as [`Model::draw`] does.
    fn relation(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        match kind {
            Kind::AddRelation => {
                if broke {
                    let r = rng.pick(&self.s.rels);
                    let text = format!(
                        "add relation {} : {} key id generate sequence",
                        r.name, r.ty
                    );
                    return Some(vec![broken(kind, text)]);
                }
                let n = self.fresh("");
                let (name, ty) = (format!("r{n}"), format!("T{n}"));
                let mut rel = Rel {
                    name: name.clone(),
                    ty: ty.clone(),
                    sum: false,
                    keys: 0,
                    header: Vec::new(),
                    ctors: vec![Ctor {
                        name: ty.clone(),
                        fields: vec![Field::new("a", Ty::Int)],
                    }],
                    retag: None,
                    set: None,
                    set_name: super::super::SET.to_owned(),
                    states: vec![Stage::Enabled],
                    uniques: Vec::new(),
                    indexes: Vec::new(),
                    checks: Vec::new(),
                    defaults: Vec::new(),
                };
                rel.uniques.clear();
                self.s.rels.push(rel);
                self.keys.add();
                let (mut out, reach, last) = match self.s.extra.next % 8 {
                    0..=4 => {
                        let f = self.fresh("f");
                        let decl = format!("{f} (k: Key {name}) -> Key {name} = k");
                        let ops = vec![
                            op(Kind::DefineFn, format!("define fn {decl}")),
                            op(Kind::RedefineFn, format!("redefine fn {decl}")),
                        ];
                        let drop = op(Kind::DropFn, format!("drop fn {f}"));
                        (ops, Some(super::Reach::RewrittenWaiting), Some(drop))
                    }
                    5 | 6 => {
                        let w = self.fresh("W");
                        let ops = vec![
                            op(
                                Kind::AddType,
                                format!("add type {w} = {{ id: Key {name} }}"),
                            ),
                            op(Kind::DropType, format!("drop type {w}")),
                        ];
                        (ops, Some(super::Reach::DroppedWaiting), None)
                    }
                    _ => (Vec::new(), None, None),
                };
                if self.s.extra.next.is_multiple_of(2) {
                    out.splice(0..0, self.transient(rng));
                }
                out.push(op(
                    Kind::AddType,
                    format!("add type {ty} = {{ id: Key {name}, a: I64 }}"),
                ));
                out.push(
                    op(
                        kind,
                        format!("add relation {name} : {ty} key id generate sequence"),
                    )
                    .with(Effect::AddRel(name))
                    .reaching(reach),
                );
                out.extend(last);
                Some(out)
            }
            Kind::DropRelation => {
                if broke {
                    let held: Vec<usize> = (0..self.s.rels.len())
                        .filter(|j| self.referenced(*j))
                        .collect();
                    let text = match rng.pick_some(&held) {
                        Some(j) => format!("drop relation {}", self.rel(*j).name),
                        None => "drop relation zz".into(),
                    };
                    return Some(vec![broken(kind, text)]);
                }
                let free: Vec<usize> = (0..self.s.rels.len())
                    .filter(|j| {
                        let r = self.rel(*j);
                        let selects_own_set = self.refs().iter().any(|(i, _, f)| {
                            *i == *j && f.ty.target() == Some((*j, &Scope::Named))
                        });
                        !self.referenced(*j)
                            && !self.tagged(*j)
                            && (super::super::SEARCH_DROP_OF_SELF_SELECTED_SET || !selects_own_set)
                            && r.checks.is_empty()
                            && !self.s.constraints.iter().any(|k| k.rel == *j)
                            && !self.s.extra.fns.iter().any(|f| f.reads == Some(*j))
                    })
                    .collect();
                if self.s.rels.len() < 2 {
                    return None;
                }
                let j = *rng.pick_some(&free)?;
                let r = self.rel(j);
                let mut out = Vec::new();
                if r.set.is_some() {
                    out.push(op(
                        Kind::DropSet,
                        format!("drop set {} of {}", r.set_name, r.name),
                    ));
                }
                out.push(
                    emptying(kind, format!("drop relation {}", r.name), r.name.clone())
                        .with(Effect::DropRel(r.name.clone())),
                );
                out.push(data(Kind::DropType, format!("drop type {}", r.ty)));
                self.remove_rel(j);
                Some(out)
            }
            Kind::RenameRelation => {
                let i = rng.below(self.s.rels.len() as u64) as usize;
                let from = self.rel(i).name.clone();
                if broke {
                    let other = &self.s.rels[(i + 1) % self.s.rels.len()].name;
                    if *other == from {
                        return None;
                    }
                    return Some(vec![broken(
                        kind,
                        format!("rename relation {from} to {other}"),
                    )]);
                }
                let bound = super::super::BOUND;
                let to = if self.s.rels.iter().all(|r| r.name != bound) {
                    bound.to_owned()
                } else {
                    self.fresh("s")
                };
                self.s.rels[i].name.clone_from(&to);
                self.keys.renamed(&from, &to);
                let text = format!("rename relation {from} to {to}");
                Some(vec![op(kind, text).with(Effect::RenameRel(from, to))])
            }
            other => unreachable!("{other:?} is no relation operator"),
        }
    }

    /// Returns the operators adding a type and its relation, then an operator on them whose
    /// inverse refuses while it is built, then dropping them.
    fn transient(&mut self, rng: &mut Rng) -> Vec<Op> {
        let n = self.fresh("");
        let (r, t, k) = (format!("r{n}"), format!("T{n}"), format!("K{n}"));
        let (a, b) = (format!("E{n}"), format!("F{n}"));
        let (p, q, m) = (format!("P{n}"), format!("Q{n}"), format!("M{n}"));
        let mut out = vec![
            op(
                Kind::AddType,
                format!(
                    "add type {t} {{ id: Key {r}, h: I64 }} = {a} {{ a: I64, b: I64 }} | {b} {{ b: I64 }}"
                ),
            ),
            op(
                Kind::AddRelation,
                format!("add relation {r} : {t} key id generate sequence"),
            ),
        ];
        let reach = Some(super::Reach::RefusedInverseOfTransient);
        let refused = match rng.below(4) {
            0 => op(Kind::DropField, format!("drop field {t}.{a}.a")),
            1 => op(Kind::MoveFieldDown, format!("move field {t}.h to {t}.{a}")),
            2 => op(
                Kind::Split,
                format!(
                    "split constructor {t}.{a} into {p} {{ a: I64, b: I64 }} | {q} {{ a: I64, b: I64 }}\n  by match\n    | {a} {{ a, b }} -> {p} {{ a: a, b: b }}"
                ),
            ),
            _ => {
                out.insert(0, op(Kind::AddType, format!("add type {k} = {a} | {b}")));
                op(
                    Kind::Merge,
                    format!(
                        "merge constructor {t}.({a} | {b}) into {m} {{ k: {k}, b: I64 }}\n  by match\n    | {a} x -> {m} {{ k: {a}, b: x.b }}\n    | {b} y -> {m} {{ k: {b}, b: y.b }}"
                    ),
                )
            }
        };
        let merged = refused.kind == Kind::Merge;
        out.push(refused.reaching(reach));
        out.push(op(Kind::DropRelation, format!("drop relation {r}")));
        out.push(op(Kind::DropType, format!("drop type {t}")));
        if merged {
            out.push(op(Kind::DropType, format!("drop type {k}")));
        }
        out
    }

    /// Draw a named set operator of `kind`, as [`Model::draw`] does.
    fn set(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        match kind {
            Kind::AddSet => {
                let i = *rng.pick_some(&self.sums())?;
                let r = self.rel(i);
                if broke {
                    let text = format!("add set {} of {} = Zz", r.set_name, r.name);
                    return Some(vec![broken(kind, text)]);
                }
                if r.set.is_some() || r.ctors.len() < 2 {
                    return None;
                }
                let pair = r.pair(rng);
                let name = self.fresh("S");
                let r = &mut self.s.rels[i];
                let text = format!(
                    "add set {name} of {} = {}",
                    r.name,
                    r.ctor_set(&pair).trim_matches(['(', ')'])
                );
                r.set = Some(pair);
                r.set_name = name;
                Some(vec![op(kind, text)])
            }
            Kind::DropSet => {
                let with: Vec<usize> = (0..self.s.rels.len())
                    .filter(|i| self.rel(*i).set.is_some())
                    .collect();
                if broke {
                    let named: Vec<usize> = with
                        .iter()
                        .copied()
                        .filter(|i| self.set_named(*i))
                        .collect();
                    let text = match rng.pick_some(&named) {
                        Some(i) => format!(
                            "drop set {} of {}",
                            self.rel(*i).set_name,
                            self.rel(*i).name
                        ),
                        None => format!("drop set Zz of {}", self.rel(0).name),
                    };
                    return Some(vec![broken(kind, text)]);
                }
                let free: Vec<usize> = with.into_iter().filter(|i| !self.set_named(*i)).collect();
                let i = *rng.pick_some(&free)?;
                let r = &mut self.s.rels[i];
                r.set = None;
                Some(vec![op(
                    kind,
                    format!("drop set {} of {}", r.set_name, r.name),
                )])
            }
            Kind::RenameSet => {
                let with: Vec<usize> = (0..self.s.rels.len())
                    .filter(|i| self.rel(*i).set.is_some())
                    .collect();
                let i = *rng.pick_some(&with)?;
                let (from, rel) = (self.rel(i).set_name.clone(), self.rel(i).name.clone());
                if broke {
                    let to = self.rel(i).ctors[0].name.clone();
                    return Some(vec![broken(
                        kind,
                        format!("rename set {from} of {rel} to {to}"),
                    )]);
                }
                let to = self.ctor_name(i);
                self.s.rels[i].set_name.clone_from(&to);
                Some(vec![op(
                    kind,
                    format!("rename set {from} of {rel} to {to}"),
                )])
            }
            other => unreachable!("{other:?} is no set operator"),
        }
    }

    /// Draw a constructor operator of `kind`, as [`Model::draw`] does.
    fn constructor(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        match kind {
            Kind::AddConstructor => {
                let i = *rng.pick_some(&self.sums())?;
                let ty = self.rel(i).ty.clone();
                if broke {
                    let c = self.rel(i).ctors[0].name.clone();
                    return Some(vec![broken(kind, format!("add constructor {ty}.{c}"))]);
                }
                let name = self.ctor_name(i);
                let fields = if rng.below(2) == 0 {
                    Vec::new()
                } else {
                    vec![Field::new(&self.fresh("g"), Ty::Int)]
                };
                let record = match fields.first() {
                    Some(f) => format!(" {{ {}: I64 }}", f.name),
                    None => String::new(),
                };
                let r = self.rel(i);
                let joins = r.set.is_some()
                    && !self.set_named(i)
                    && !r
                        .checks
                        .iter()
                        .any(|k| k.ctors.is_some() && k.ctors == r.set)
                    && rng.below(2) == 0;
                let mut text = format!("add constructor {ty}.{name}{record}");
                if joins {
                    text.push_str(&format!("\n  joins {}.{}", r.name, r.set_name));
                }
                let matcher = (rng.below(4) != 0).then(|| {
                    let f = self.fresh("f");
                    let names: Vec<String> =
                        self.rel(i).ctors.iter().map(|c| c.name.clone()).collect();
                    let body = |names: &[String]| {
                        let arms: String =
                            names.iter().map(|c| format!("\n  | {c} x -> 1")).collect();
                        format!("{f} (t: {ty}) -> I64 = match t{arms}")
                    };
                    let mut all = names.clone();
                    all.push(name.clone());
                    (f.clone(), body(&names), body(&all))
                });
                let r = &mut self.s.rels[i];
                let c = r.ctors.len();
                r.ctors.push(Ctor { name, fields });
                r.states.push(Stage::Declared);
                if joins && let Some(set) = &mut r.set {
                    set.push(c);
                }
                let Some((f, without, with)) = matcher else {
                    return Some(vec![op(kind, text)]);
                };
                let mut out = vec![
                    op(Kind::DefineFn, format!("define fn {without}")),
                    op(kind, text).reaching(Some(super::Reach::ArmLater)),
                ];
                let before = self.clone();
                self.unbuilt = true;
                let own = format!("split constructor {ty}.");
                let split = self
                    .split(rng, false)
                    .filter(|ops| ops.len() == 1 && !ops[0].text().starts_with(&own));
                self.unbuilt = false;
                match split {
                    Some(ops) => out.extend(ops),
                    None => *self = before,
                }
                out.push(op(Kind::RedefineFn, format!("redefine fn {with}")));
                out.push(op(Kind::DropFn, format!("drop fn {f}")));
                Some(out)
            }
            Kind::RemoveConstructor => {
                let mut can = Vec::new();
                let mut enabled = Vec::new();
                for i in self.sums() {
                    for c in 0..self.rel(i).ctors.len() {
                        let st = self.rel(i).states[c];
                        if st == Stage::Enabled {
                            enabled.push((i, c));
                        }
                        let earlier = matches!(st, Stage::Deprecated(v) if v < self.version);
                        if earlier && self.removable(i, c) {
                            can.push((i, c));
                        }
                    }
                }
                if broke {
                    let (i, c) = *rng.pick_some(&enabled)?;
                    let text = format!("remove constructor {}", self.ctor_path(i, c));
                    return Some(vec![broken(kind, text)]);
                }
                let can =
                    Self::steered(can.iter().map(|&(i, c)| ((i, c), self.tagged(i))).collect());
                let (i, c) = *rng.pick_some(&can)?;
                let text = format!("remove constructor {}", self.ctor_path(i, c));
                let target = self.rel(i).target(Some(&vec![c]));
                let effect = Effect::RemoveCtor {
                    rel: self.rel(i).name.clone(),
                    ty: self.rel(i).ty.clone(),
                    ctor: self.rel(i).ctors[c].name.clone(),
                };
                let mut out: Vec<Op> = self.tag_of(rng, i, &[c], false).into_iter().collect();
                self.replace(i, &[c], Vec::new(), Vec::new());
                out.push(emptying(kind, text, target).with(effect));
                Some(out)
            }
            Kind::RenameConstructor => {
                let i = *rng.pick_some(&self.sums())?;
                let c = rng.below(self.rel(i).ctors.len() as u64) as usize;
                let path = self.ctor_path(i, c);
                if broke {
                    let r = self.rel(i);
                    let to = r.ctors[(c + 1) % r.ctors.len()].name.clone();
                    return Some(vec![broken(
                        kind,
                        format!("rename constructor {path} to {to}"),
                    )]);
                }
                let to = self.ctor_name(i);
                let effect = Effect::RenameCtor {
                    rel: self.rel(i).name.clone(),
                    from: self.rel(i).ctors[c].name.clone(),
                    to: to.clone(),
                };
                self.s.rels[i].ctors[c].name.clone_from(&to);
                let text = format!("rename constructor {path} to {to}");
                Some(vec![op(kind, text).with(effect)])
            }
            other => unreachable!("{other:?} is no constructor operator"),
        }
    }
}
