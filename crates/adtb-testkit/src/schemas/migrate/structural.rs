use super::super::{Action, Clause, Ctor, Field, Scope, Stage, Ty, literal};
use super::rows::Effect;
use super::{Kind, Model, Op, Reach, broken, data, op, unimaged};
use crate::Rng;

/// Two constructors of a relation: the relation and the two constructors.
type Pair = (usize, usize, usize);

impl Model {
    pub(super) fn split(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let can = self.replaceable();
        let mut before = Vec::new();
        let uncarried = broke && self.want == Some(Reach::UncarriedDefault);
        let shape = self.split_shape(rng, &can, (broke, uncarried), &mut before);
        let can = self.splittable(can);
        let renaming = !broke
            && !self.unbuilt
            && matches!(self.want, Some(Reach::RenamedCarry | Reach::RenamedDefault));
        let renamable: Vec<(usize, usize)> = can
            .iter()
            .copied()
            .filter(|&(i, c)| renaming && self.renamable(i, c))
            .collect();
        let (i, c) = match shape {
            Some(p) => p,
            None if !renamable.is_empty() => *rng.pick(&renamable),
            None => *rng.pick_some(&can)?,
        };
        let wrong = broke && shape.is_none();
        let renames = renaming || self.version % 3 == 1;
        if !broke && !self.unbuilt && shape.is_none() && renames && self.renamable(i, c) {
            return Some(self.renamed_split(rng, (i, c)));
        }
        let mut out = if broke { Vec::new() } else { self.seeded(i, c) };
        let steer = self.unique_alone(i, c);
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
            Some(b) if shape.is_some() => 2 + b,
            Some(b)
                if (steer || rng.below(2) == 0) && self.replaces_under(i, &[c], 2, Kind::Split) =>
            {
                2 + b
            }
            _ => 1,
        };
        let names: Vec<String> = if wrong {
            let r = self.rel(i);
            vec![r.ctors[(c + 1) % r.ctors.len()].name.clone()]
        } else if products == 1 {
            vec![self.ctor_name(i)]
        } else {
            let a = self.ctor_name(i);
            vec![a, self.ctor_name(i)]
        };
        if wrong && names[0] == src.name {
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
        if shape.is_some() {
            let defaulted = flag.is_some_and(|b| {
                let flag = &src.fields[b].name;
                self.rel(i)
                    .defaults
                    .iter()
                    .any(|(d, f, _)| *d == c && f == flag)
            });
            let reach = defaulted.then_some(Reach::UncarriedDefault);
            before.push(unimaged(Kind::Split, text).reaching(reach));
            return Some(before);
        }
        if broke {
            return Some(vec![broken(Kind::Split, text)]);
        }
        out.extend(self.tag_of(rng, i, &[c], products > 1));
        let effect = Effect::Split {
            rel: self.rel(i).name.clone(),
            ty: self.rel(i).ty.clone(),
            ctor: src.name.clone(),
            flag: (products > 1).then(|| src.fields[products - 2].name.clone()),
            products: names,
            renamed: Vec::new(),
        };
        let states = vec![state; made.len()];
        let unbuilt =
            (self.unbuilt && self.unique_scoped(i, c)).then_some(Reach::ReplacedWhileUnbuilt);
        self.replace(i, &[c], made, states);
        out.push(op(Kind::Split, text).with(effect).reaching(unbuilt));
        Some(out)
    }

    /// Returns a split of constructor `c` of relation `i` into one product carrying each field
    /// under a fresh name, after an `add field` of a tag naming `c` on half its draws, applying
    /// them to the model.
    fn renamed_split(&mut self, rng: &mut Rng, (i, c): (usize, usize)) -> Vec<Op> {
        let mut out: Vec<Op> = self.tag_of(rng, i, &[c], false).into_iter().collect();
        let r = self.rel(i);
        let bare = r.ctors[c]
            .fields
            .iter()
            .find(|f| f.ty.literal() && !r.has_default(c, &f.name))
            .cloned();
        if self.want == Some(Reach::RenamedDefault)
            && let Some(f) = bare
        {
            out.push(self.defaulted(i, c, &f));
        }
        let path = self.ctor_path(i, c);
        let name = self.ctor_name(i);
        let src = self.rel(i).ctors[c].clone();
        let renamed: Vec<(String, String)> = src
            .fields
            .iter()
            .map(|f| (f.name.clone(), self.field_name(i)))
            .collect();
        let fields: Vec<Field> = src
            .fields
            .iter()
            .zip(&renamed)
            .map(|(f, (_, to))| Field {
                name: to.clone(),
                ..f.clone()
            })
            .collect();
        let decls: Vec<String> = fields
            .iter()
            .map(|f| self.s.field_decl(self.rel(i), f))
            .collect();
        let carry: Vec<String> = renamed
            .iter()
            .map(|(from, to)| format!("{to}: c.{from}"))
            .collect();
        let text = format!(
            "split constructor {path} into {name} {{ {} }}\n  by match\n    | {} c -> {name} {{ {} }}",
            decls.join(", "),
            src.name,
            carry.join(", ")
        );
        let r = &mut self.s.rels[i];
        for k in r.uniques.iter_mut().chain(&mut r.indexes) {
            if let Some((_, to)) = renamed.iter().find(|(from, _)| *from == k.field) {
                k.field.clone_from(to);
            }
        }
        let defaulted = r.defaults.iter().any(|(d, _, _)| *d == c);
        for (d, g, _) in &mut r.defaults {
            if let Some((_, to)) = renamed.iter().find(|(from, _)| *from == *g)
                && *d == c
            {
                g.clone_from(to);
            }
        }
        let effect = Effect::Split {
            rel: r.name.clone(),
            ty: r.ty.clone(),
            ctor: src.name,
            flag: None,
            products: vec![name.clone()],
            renamed,
        };
        let state = r.states[c];
        let made = Ctor { name, fields };
        self.replace(i, &[c], vec![made], vec![state]);
        let reach = match defaulted {
            true => Reach::RenamedDefault,
            false => Reach::RenamedCarry,
        };
        out.push(op(Kind::Split, text).with(effect).reaching(Some(reach)));
        out
    }

    /// Returns whether a split of constructor `c` of relation `i` may carry each field under a
    /// new name: no check or constraint names a field of `c`, and every unique or index naming a
    /// field of `c` is scoped to `c` alone.
    fn renamable(&self, i: usize, c: usize) -> bool {
        let r = self.rel(i);
        let own = |f: &str| r.ctors[c].field(f).is_some();
        let alone = |k: &Clause| !own(&k.field) || k.ctors.as_deref() == Some(&[c][..]);
        !r.ctors[c].fields.is_empty()
            && r.uniques.iter().chain(&r.indexes).all(alone)
            && r.checks
                .iter()
                .all(|k| k.ctors.as_ref().is_some_and(|cs| !cs.contains(&c)))
            && self
                .s
                .constraints
                .iter()
                .all(|k| k.rel != i || !k.ctors.contains(&c))
    }

    /// Returns whether a unique of relation `i` is scoped to constructor `c` alone.
    fn unique_alone(&self, i: usize, c: usize) -> bool {
        self.rel(i)
            .uniques
            .iter()
            .any(|k| k.ctors.as_deref() == Some(&[c][..]))
    }

    /// Returns whether a unique of relation `i` is scoped to constructors that include `c`.
    fn unique_scoped(&self, i: usize, c: usize) -> bool {
        self.rel(i)
            .uniques
            .iter()
            .any(|k| k.ctors.as_ref().is_some_and(|cs| cs.contains(&c)))
    }

    /// Returns whether constructor `c` of relation `i` holds a flag a split may route by.
    fn flagged(&self, (i, c): (usize, usize)) -> bool {
        self.rel(i).ctors[c]
            .fields
            .iter()
            .any(|f| f.ty == Ty::Bool && f.action == Action::Unstated)
    }

    /// Returns the constructors of `can` a split has an exact image for, steered to those
    /// [`Model::steered`] prefers: one that splits in two or seeds rows, or while the
    /// declarations do not build, one a unique names that seeds none.
    fn splittable(&self, can: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
        let can: Vec<((usize, usize), bool)> = can
            .into_iter()
            .filter_map(|(i, c)| {
                if !self.replaces_under(i, &[c], 1, Kind::Split) {
                    return None;
                }
                let two = self.flagged((i, c)) && self.replaces_under(i, &[c], 2, Kind::Split);
                let seeds = !self.clone().seeded(i, c).is_empty();
                if self.unbuilt {
                    return Some(((i, c), !seeds && self.unique_scoped(i, c)));
                }
                Some(((i, c), two || seeds))
            })
            .collect();
        Model::steered(can)
    }

    /// Returns the constructor a broken split by its flag has no exact image for, its operators
    /// pushed to `before`: with `uncarried`, one [`Model::defaulted_flag`] adds; else, on half
    /// the draws, one [`Model::unimaged_split`] picks. `None` for any other split.
    fn split_shape(
        &mut self,
        rng: &mut Rng,
        can: &[(usize, usize)],
        (broke, uncarried): (bool, bool),
        before: &mut Vec<Op>,
    ) -> Option<(usize, usize)> {
        if uncarried {
            self.defaulted_flag(rng, before)
        } else if broke && rng.below(2) == 0 {
            self.unimaged_split(rng, can, before)
        } else {
            None
        }
    }

    /// Returns a constructor of `can` whose split by its flag has no exact image under what
    /// names it, else one [`Model::defaulted_flag`] adds with its operators pushed to `before`.
    fn unimaged_split(
        &mut self,
        rng: &mut Rng,
        can: &[(usize, usize)],
        before: &mut Vec<Op>,
    ) -> Option<(usize, usize)> {
        let shapes: Vec<(usize, usize)> = can
            .iter()
            .copied()
            .filter(|&(i, c)| self.flagged((i, c)) && !self.replaces_under(i, &[c], 2, Kind::Split))
            .collect();
        match rng.pick_some(&shapes) {
            Some(&p) => Some(p),
            None => self.defaulted_flag(rng, before),
        }
    }

    /// Returns the pairs of constructors an operator of `kind` may merge: those whose merge has
    /// an exact image under what names them, then those whose merge has none.
    fn mergeable(&self, kind: Kind) -> (Vec<Pair>, Vec<Pair>) {
        let free = self.replaceable();
        let mut pairs = Vec::new();
        let mut shapes = Vec::new();
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
                if fits && !apart && !actions {
                    if self.replaces_under(i, &[c, d], 1, kind) {
                        pairs.push((i, c, d));
                    } else {
                        shapes.push((i, c, d));
                    }
                }
            }
        }
        (pairs, shapes)
    }

    pub(super) fn merge(&mut self, rng: &mut Rng, kind: Kind, broke: bool) -> Option<Vec<Op>> {
        let (pairs, shapes) = self.mergeable(kind);
        let mut out = Vec::new();
        let nested = kind == Kind::Nest && self.want == Some(Reach::NestedDefault);
        let want = broke && (nested || rng.below(2) == 0);
        let shape = if want && !nested {
            rng.pick_some(&shapes).copied()
        } else {
            None
        };
        let steer = want && shape.is_none();
        let under = !broke
            && kind == Kind::Merge
            && (self.want == Some(Reach::MergeUnderCheck) || rng.below(2) == 0);
        let held = if (steer || under) && (kind == Kind::Merge || nested) {
            Some(self.fresh("g"))
        } else {
            None
        };
        let (i, c, d) = match shape.or_else(|| rng.pick_some(&pairs).copied()) {
            Some(p) if !steer && !under => p,
            _ => self.fresh_pair(rng, steer, held.as_deref(), &mut out)?,
        };
        if steer {
            match (held.clone(), self.header_key(i)) {
                (Some(g), _) => out.push(self.defaulted(i, c, &Field::new(&g, Ty::Int))),
                (None, Some(h)) => out.push(self.unique_in(i, c, h)),
                (None, None) => {}
            }
        }
        if under && let Some(g) = &held {
            out.push(self.checked(i, c, g.clone()));
        }
        let shape = shape
            .or_else(|| (steer && !self.replaces_under(i, &[c, d], 1, kind)).then_some((i, c, d)));
        let wrong = broke && shape.is_none();
        let ty = self.rel(i).ty.clone();
        let (cn, dn) = (
            self.rel(i).ctors[c].name.clone(),
            self.rel(i).ctors[d].name.clone(),
        );
        let tag = self.fresh("K");
        let product = self.ctor_name(i);
        let record = |n: &String| match (kind, &held) {
            (Kind::Nest, Some(g)) => format!("{n} {{ {g}: I64 }}"),
            _ => n.clone(),
        };
        let tag_ctors = if wrong && kind == Kind::Nest {
            vec![record(&cn), "Zz".to_owned()]
        } else {
            vec![record(&cn), record(&dn)]
        };
        let add_type = op(
            Kind::AddType,
            format!("add type {tag} = {}", tag_ctors.join(" | ")),
        );
        let fields = self.rel(i).ctors[c].fields.clone();
        let field = self.fresh(if kind == Kind::Merge { "k" } else { "f" });
        let mut decl: Vec<String> = Vec::new();
        if !(wrong && kind == Kind::Merge) {
            decl.push(format!("{field}: {tag}"));
        }
        if kind == Kind::Merge || held.is_none() {
            decl.extend(fields.iter().map(|f| self.s.field_decl(self.rel(i), f)));
        }
        let verb = if kind == Kind::Merge { "merge" } else { "nest" };
        let text = format!(
            "{verb} constructor {ty}.({cn} | {dn}) into {product} {{ {} }}",
            decl.join(", ")
        );
        if shape.is_some() {
            let r = self.rel(i);
            let defaulted = nested && r.defaults.iter().any(|x| x.0 == c || x.0 == d);
            out.extend([
                add_type,
                unimaged(kind, text).reaching(defaulted.then_some(Reach::NestedDefault)),
            ]);
            return Some(out);
        }
        if broke {
            out.extend([add_type, broken(kind, text)]);
            return Some(out);
        }
        out.push(add_type);
        out.extend(self.tag_of(rng, i, &[c, d], false));
        let effect = Effect::Merge {
            rel: self.rel(i).name.clone(),
            ty: self.rel(i).ty.clone(),
            sources: vec![cn, dn],
            product: product.clone(),
            field: field.clone(),
        };
        let states = &self.rel(i).states;
        let reach = if states[c] != states[d] {
            Some(Reach::MergeAcrossStages)
        } else {
            under.then_some(Reach::MergeUnderCheck)
        };
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
        let merged = op(kind, text).with(effect).reaching(reach);
        out.push(merged);
        Some(out)
    }

    /// Returns two constructors a merge or nest steers to, added to a relation of a sum with
    /// their operators pushed to `out`: each holding integer field `held` when given, with a
    /// third between them on even versions, the first enabled on every third version. A `steer`
    /// with no `held` picks a relation with a header key when one has it.
    fn fresh_pair(
        &mut self,
        rng: &mut Rng,
        steer: bool,
        held: Option<&str>,
        out: &mut Vec<Op>,
    ) -> Option<Pair> {
        let sums = self.sums();
        let headed: Vec<usize> = sums
            .iter()
            .copied()
            .filter(|&i| held.is_none() && steer && self.header_key(i).is_some())
            .collect();
        let i = *rng.pick_some(if headed.is_empty() { &sums } else { &headed })?;
        let ty = self.rel(i).ty.clone();
        let at = self.rel(i).ctors.len();
        let apart = self.version.is_multiple_of(2);
        let fields: Vec<Field> = held.iter().map(|g| Field::new(g, Ty::Int)).collect();
        let record = match fields.first() {
            Some(f) => format!(" {{ {}: I64 }}", f.name),
            None => String::new(),
        };
        for _ in 0..2 + usize::from(apart) {
            let name = self.ctor_name(i);
            out.push(op(
                Kind::AddConstructor,
                format!("add constructor {ty}.{name}{record}"),
            ));
            let r = &mut self.s.rels[i];
            r.ctors.push(Ctor {
                name,
                fields: fields.clone(),
            });
            r.states.push(Stage::Declared);
        }
        if self.version.is_multiple_of(3) {
            out.push(op(Kind::Enable, format!("enable {}", self.ctor_ref(i, at))));
            self.s.rels[i].states[at] = Stage::Enabled;
        }
        Some((i, at, at + 1 + usize::from(apart)))
    }

    pub(super) fn unnest(&mut self, rng: &mut Rng, broke: bool) -> Option<Vec<Op>> {
        let mut out = Vec::new();
        let mut can = self.unnestable();
        if can.is_empty() && !broke {
            out = self.merge(rng, Kind::Nest, false)?;
            can = self.unnestable();
        }
        if broke && rng.below(2) == 0 {
            let mut m = self.clone();
            let mut shapes = m.nestings(false);
            if shapes.is_empty() {
                out = m.merge(rng, Kind::Nest, false).unwrap_or_default();
                if let Some(&(i, c)) = rng.pick_some(&m.unnestable()) {
                    let f = m.rel(i).ctors[c].fields[0].clone();
                    out.push(m.defaulted(i, c, &f));
                    shapes = m.nestings(false);
                }
            }
            if let Some(&(i, c)) = rng.pick_some(&shapes) {
                let text = format!("unnest constructor {}", m.ctor_path(i, c));
                out.push(unimaged(Kind::Unnest, text));
                *self = m;
                return Some(out);
            }
            out.clear();
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
        out.extend(self.tag_of(rng, i, &[c], true));
        let effect = Effect::Unnest {
            rel: self.rel(i).name.clone(),
            ty: self.rel(i).ty.clone(),
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

    /// Add to a sum relation a constructor of one flag field with a default, which a split by
    /// the flag drops, pushing the operators to `out`. Returns the relation and constructor.
    fn defaulted_flag(&mut self, rng: &mut Rng, out: &mut Vec<Op>) -> Option<(usize, usize)> {
        let i = *rng.pick_some(&self.sums())?;
        let ty = self.rel(i).ty.clone();
        let name = self.ctor_name(i);
        let flag = Field::new(&self.fresh("b"), Ty::Bool);
        out.push(op(
            Kind::AddConstructor,
            format!("add constructor {ty}.{name} {{ {}: Bool }}", flag.name),
        ));
        let r = &mut self.s.rels[i];
        r.ctors.push(Ctor {
            name,
            fields: vec![flag.clone()],
        });
        r.states.push(Stage::Declared);
        let c = self.rel(i).ctors.len() - 1;
        out.push(self.defaulted(i, c, &flag));
        Some((i, c))
    }

    /// Returns the `add default` of field `f` of constructor `c` of relation `i`, adding it to
    /// the model.
    fn defaulted(&mut self, i: usize, c: usize, f: &Field) -> Op {
        let v = literal(&f.ty, false);
        let r = self.rel(i);
        let path = format!("{}.{}.{}", r.name, r.ctors[c].name, f.name);
        self.s.rels[i].defaults.push((c, f.name.clone(), v.clone()));
        op(Kind::AddDefault, format!("add default {path} = {v}"))
    }

    /// Returns the `add check` of integer field `f` scoped to constructor `c` of relation `i`,
    /// adding it to the model.
    fn checked(&mut self, i: usize, c: usize, f: String) -> Op {
        let at = self.rel(i).target(Some(&vec![c]));
        let name = self.fresh("k");
        let text = format!("add check {name} on {at} ({f} >= (-2))");
        self.s.rels[i].checks.push(Clause {
            field: f,
            ctors: Some(vec![c]),
            name,
        });
        data(Kind::AddCheck, text)
    }

    /// Returns a header field of relation `i` past its key that a unique may name.
    fn header_key(&self, i: usize) -> Option<String> {
        let r = self.rel(i);
        let f = r.plain_header().iter().find(|f| f.ty.keyed())?;
        Some(f.name.clone())
    }

    /// Returns the `add unique` of header field `f` of relation `i` scoped to constructor `c`,
    /// adding it to the model.
    fn unique_in(&mut self, i: usize, c: usize, f: String) -> Op {
        let at = self.rel(i).target(Some(&vec![c]));
        let text = format!("add unique ({f}) in {at}");
        self.s.rels[i].uniques.push(Clause {
            field: f,
            ctors: Some(vec![c]),
            name: String::new(),
        });
        data(Kind::AddUnique, text)
    }

    /// Returns each constructor holding one field of a sum declared by a nest, whose
    /// constructors are free names of its relation.
    pub(super) fn unnestable(&self) -> Vec<(usize, usize)> {
        self.nestings(true)
    }

    /// Returns each constructor [`Model::unnestable`] would return but for the clauses,
    /// constraints and defaults naming it, whose unnest has an exact image when `imaged`, else
    /// lacks one.
    fn nestings(&self, imaged: bool) -> Vec<(usize, usize)> {
        let mut can = Vec::new();
        for (i, c) in self.replaceable() {
            let ctor = &self.rel(i).ctors[c];
            if let [f] = ctor.fields.as_slice()
                && let Ty::Tag(_, cs) = &f.ty
                && cs
                    .iter()
                    .all(|n| self.rel(i).ctors.iter().all(|d| d.name != *n))
                && cs.iter().all(|n| *n != self.rel(i).set_name)
                && self.replaces_under(i, &[c], cs.len(), Kind::Unnest) == imaged
                && !self
                    .rel(i)
                    .checks
                    .iter()
                    .any(|k| k.ctors.as_ref().is_some_and(|ks| ks.contains(&c)))
            {
                can.push((i, c));
            }
        }
        can
    }
}
