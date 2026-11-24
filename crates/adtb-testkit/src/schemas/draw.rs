//! Statements drawn over a generated schema.

use std::collections::BTreeMap;

use super::{Action, Field, Mark, Schema, Scope, Stage, Stmt, Ty};
use crate::Rng;

/// The binders of an insert's query, each with the condition naming its one row, and the key
/// the row's last reference to each relation names.
#[derive(Default)]
struct Bound {
    binds: Vec<(String, String)>,
    keys: BTreeMap<usize, String>,
}

/// The keys each relation's inserts have issued, and the keys of rows the inserts into a
/// relation of a composite key give, by relation.
#[derive(Clone, Debug, Default)]
pub(super) struct Keys {
    issued: Vec<u64>,
    given: Vec<Vec<String>>,
}

impl Keys {
    /// Count a relation added after the others, none of its keys issued.
    pub(super) fn add(&mut self) {
        self.issued.push(0);
        self.given.push(Vec::new());
    }

    /// Forget relation `j`, the relations after it moving down one.
    pub(super) fn remove(&mut self, j: usize) {
        self.issued.remove(j);
        self.given.remove(j);
    }

    /// Name relation `from` as `to` in the keys a key of another relation holds.
    pub(super) fn renamed(&mut self, from: &str, to: &str) {
        let (from, to) = (format!(" {from}.key("), format!(" {to}.key("));
        for k in self.given.iter_mut().flatten() {
            *k = k.replace(&from, &to);
        }
    }

    /// Forget the keys of the rows inserts into relation `j` gave.
    pub(super) fn rekeyed(&mut self, j: usize) {
        self.given[j].clear();
    }
}

/// A drawer of statements, holding the keys its inserts have issued.
pub(super) struct Draw<'s> {
    s: &'s Schema,
    issued: Vec<u64>,
    given: Vec<Vec<String>>,
}

impl<'s> Draw<'s> {
    pub(super) fn new(s: &'s Schema) -> Draw<'s> {
        Draw::resume(
            s,
            Keys {
                issued: vec![0; s.rels.len()],
                given: vec![Vec::new(); s.rels.len()],
            },
        )
    }

    /// Returns a drawer of statements over `s` following those that issued `keys`.
    pub(super) fn resume(s: &'s Schema, keys: Keys) -> Draw<'s> {
        Draw {
            s,
            issued: keys.issued,
            given: keys.given,
        }
    }

    pub(super) fn keys(self) -> Keys {
        Keys {
            issued: self.issued,
            given: self.given,
        }
    }

    pub(super) fn stmt(&mut self, rng: &mut Rng) -> Stmt {
        let i = rng.below(self.s.rels.len() as u64) as usize;
        match rng.below(20) {
            0..=6 => self.insert(rng, i),
            7 => self.delete(rng, i),
            8..=10 => self.delete_referenced(rng),
            11 | 12 => self.retag(rng, i),
            13 | 14 => self.update(rng, i),
            15 => self.pair(rng),
            16 => self.set_unique(rng),
            _ => self.query(rng, i),
        }
    }

    fn int(rng: &mut Rng) -> String {
        match rng.below(7) as i64 - 3 {
            n if n < 0 => format!("({n})"),
            n => n.to_string(),
        }
    }

    fn sum(rng: &mut Rng, k: usize) -> String {
        match (k, rng.below(3)) {
            (0, 0) => "Z".into(),
            (0, 1) => format!("S {{ v: {} }}", rng.below(2)),
            (0, _) => format!(
                "W {{ t: {}, o: {} }}",
                rng.pick(&[r#""a""#, r#""b""#]),
                Self::opt_int(rng)
            ),
            (_, 0) => "M".into(),
            _ => format!("N {{ w: {} }}", Self::sum(rng, 0)),
        }
    }

    fn opt_int(rng: &mut Rng) -> String {
        if rng.below(3) == 0 {
            "None".into()
        } else {
            format!("Some {{ value: {} }}", Self::int(rng))
        }
    }

    /// Returns the expression of a key of relation `j`, of a row that may not exist.
    fn key(&self, rng: &mut Rng, j: usize) -> String {
        let r = &self.s.rels[j];
        if r.keys == 0 {
            let k = match self.issued[j] {
                0 => 1,
                n if rng.below(8) == 0 => n + 1,
                n => 1 + rng.below(n),
            };
            return format!("{}.key({k})", r.name);
        }
        match rng.pick_some(&self.given[j]) {
            Some(k) if rng.below(8) != 0 => format!("{}.key({k})", r.name),
            _ => format!("{}.key({})", r.name, self.key_fields(rng, j).join(", ")),
        }
    }

    /// Returns the expression of a key of relation `j`, of a row that may not exist.
    pub(super) fn some_key(&self, rng: &mut Rng, j: usize) -> String {
        self.key(rng, j)
    }

    /// Returns the key fields of a row of relation `j` of a composite key, each `f: v`.
    fn key_fields(&self, rng: &mut Rng, j: usize) -> Vec<String> {
        let r = &self.s.rels[j];
        r.header[..r.keys]
            .iter()
            .map(|f| format!("{}: {}", f.name, self.key_part(rng, &f.ty)))
            .collect()
    }

    /// Returns a value of key field type `ty`, from a domain small enough that keys repeat.
    fn key_part(&self, rng: &mut Rng, ty: &Ty) -> String {
        match ty {
            Ty::Int => rng.below(2).to_string(),
            Ty::Sum(0) => rng.pick(&["Z", "S { v: 0 }"]).to_string(),
            Ty::Sum(_) => rng.pick(&["M", "N { w: Z }"]).to_string(),
            ty => self.value(rng, ty),
        }
    }

    /// Returns a value of `ty` for a row of an insert, a reference often naming the key the
    /// row's last reference to its relation names, and a reference to a narrowed constructor set
    /// as the key of a binder of `b` over that set.
    fn bound(&self, rng: &mut Rng, ty: &Ty, b: &mut Bound) -> String {
        let Some((j, scope)) = ty.target() else {
            return self.value(rng, ty);
        };
        if matches!(ty, Ty::OptRef(..)) && rng.below(4) == 0 {
            return "None".into();
        }
        let key = match b.keys.get(&j) {
            Some(k) if rng.below(2) == 0 => k.clone(),
            _ => self.key(rng, j),
        };
        b.keys.insert(j, key.clone());
        let r = &self.s.rels[j];
        let held = if ty.narrowed() {
            let y = format!("y{}", b.binds.len());
            let over = match scope {
                Scope::Ctors(cs) => r.target(Some(cs)),
                Scope::Named | Scope::All => r.target(r.set.as_ref()),
            };
            b.binds
                .push((format!("{y} in {over}"), format!("key {y} == {key}")));
            format!("key {y}")
        } else {
            key
        };
        match ty {
            Ty::OptRef(..) => format!("Some {{ value: {held} }}"),
            _ => held,
        }
    }

    fn value(&self, rng: &mut Rng, ty: &Ty) -> String {
        match ty {
            Ty::Int => Self::int(rng),
            Ty::Text => rng.pick(&[r#""a""#, r#""b""#, r#""""#]).to_string(),
            Ty::Bool => rng.pick(&["true", "false"]).to_string(),
            Ty::OptInt => Self::opt_int(rng),
            Ty::Sum(k) => Self::sum(rng, *k),
            Ty::Tag(_, cs) => rng.pick(cs).clone(),
            Ty::TagOf(j) => {
                let r = &self.s.rels[*j];
                r.ctors[*rng.pick_some(&r.enabled()).unwrap_or(&0)]
                    .name
                    .clone()
            }
            Ty::Ref(j, _) => self.key(rng, *j),
            Ty::OptRef(j, _) => match rng.below(4) {
                0 => "None".into(),
                _ => format!("Some {{ value: {} }}", self.key(rng, *j)),
            },
        }
    }

    pub(super) fn insert(&mut self, rng: &mut Rng, i: usize) -> Stmt {
        let r = &self.s.rels[i];
        let open = r.enabled();
        let acting: Vec<usize> = open
            .iter()
            .copied()
            .filter(|c| {
                r.ctors[*c]
                    .fields
                    .iter()
                    .any(|f| matches!(f.action, Action::Retag | Action::Clear))
            })
            .collect();
        let c = match rng.pick_some(&acting) {
            Some(c) if rng.below(2) == 0 => *c,
            _ => match rng.pick_some(&open) {
                Some(c) => *c,
                None => return self.query(rng, i),
            },
        };
        self.insert_into(rng, i, c, None)
    }

    /// Returns an insert of a row of constructor `c` of relation `i`. With `fixed`, the field it
    /// names holds the value it gives and every optional reference of the constructor `None`.
    fn insert_into(
        &mut self,
        rng: &mut Rng,
        i: usize,
        c: usize,
        fixed: Option<(&str, &str)>,
    ) -> Stmt {
        let r = &self.s.rels[i];
        let mut fields = Vec::new();
        let mut marks = Vec::new();
        let mut bound = Bound::default();
        if r.keys == 0 {
            let k = if self.issued[i] > 0 && fixed.is_none() && rng.below(8) == 0 {
                1 + rng.below(self.issued[i])
            } else {
                self.issued[i] += 1;
                self.issued[i]
            };
            fields.push(format!("id: {}.key({k})", r.name));
        }
        if r.keys > 0 {
            let key = self.key_fields(rng, i);
            if r.header[..r.keys]
                .iter()
                .any(|f| matches!(f.ty, Ty::Sum(_)))
            {
                marks.push(Mark::SumKey);
            }
            self.given[i].push(key.join(", "));
            fields.extend(key);
        }
        let r = &self.s.rels[i];
        for f in r.plain_header() {
            let v = self.bound(rng, &f.ty, &mut bound);
            fields.push(format!("{}: {v}", f.name));
        }
        for f in &r.ctors[c].fields {
            if let Some((name, v)) = fixed
                && name == f.name
            {
                fields.push(format!("{name}: {v}"));
                continue;
            }
            if r.has_default(c, &f.name) && rng.below(2) == 0 {
                continue;
            }
            let v = match f.ty {
                Ty::OptRef(..) if fixed.is_some() => "None".to_owned(),
                _ => self.bound(rng, &f.ty, &mut bound),
            };
            if r.sum
                && v != "None"
                && let Some((j, scope)) = f.ty.target()
                && j != i
                && self.s.rels[j].sum
                && *scope != Scope::All
            {
                marks.push(Mark::CrossSum);
            }
            fields.push(format!("{}: {v}", f.name));
        }
        let row = format!("{{ {} }}", fields.join(", "));
        let into = r.target(Some(&vec![c]));
        let mut s = Stmt::write(if bound.binds.is_empty() {
            format!("insert {into} {row}")
        } else {
            let (gens, conds): (Vec<String>, Vec<String>) = bound.binds.into_iter().unzip();
            format!(
                "insert {into} from (from {} where {} select {row})",
                gens.join(", "),
                conds.join(" and ")
            )
        });
        s.marks = marks;
        s
    }

    /// Returns two inserts as one statement, into two constructors of a unique over a
    /// constructor set, giving its field one value; an insert when no relation has such a unique.
    fn set_unique(&mut self, rng: &mut Rng) -> Stmt {
        let schema = self.s;
        let mut uniques = Vec::new();
        for (i, r) in schema.rels.iter().enumerate() {
            for u in &r.uniques {
                if let Some(cs) = u.ctors.as_ref().filter(|cs| cs.len() > 1)
                    && let Some(f) = r.ctors[cs[0]].field(&u.field)
                    && matches!(f.ty, Ty::Int | Ty::Text)
                {
                    uniques.push((i, cs, f));
                }
            }
        }
        let Some(&(i, cs, f)) = rng.pick_some(&uniques) else {
            let i = rng.below(self.s.rels.len() as u64) as usize;
            return self.insert(rng, i);
        };
        let v = self.value(rng, &f.ty);
        let k = rng.below(cs.len() as u64) as usize;
        let (a, b) = (cs[k], cs[(k + 1) % cs.len()]);
        let one = self.insert_into(rng, i, a, Some((&f.name, &v)));
        let key = self.given[i].last().cloned();
        let mut two = self.insert_into(rng, i, b, Some((&f.name, &v)));
        for _ in 0..4 {
            if self.given[i].last() != key.as_ref() || key.is_none() {
                break;
            }
            self.given[i].pop();
            two = self.insert_into(rng, i, b, Some((&f.name, &v)));
        }
        let mut s = Stmt::write(format!("({}, {})", one.text, two.text));
        s.marks = one.marks.into_iter().chain(two.marks).collect();
        s
    }

    fn delete(&mut self, rng: &mut Rng, i: usize) -> Stmt {
        let r = &self.s.rels[i];
        let open = r.enabled();
        let text = if r.sum && !open.is_empty() && rng.below(4) == 0 {
            let c = *rng.pick(&open);
            format!("delete x in {}", r.target(Some(&vec![c])))
        } else {
            format!("delete x in {} where key x == {}", r.name, self.key(rng, i))
        };
        let mut s = Stmt::write(text);
        s.deletes.push(r.name.clone());
        s
    }

    /// Returns a delete of every row a drawn reference field holds the key of, the field often
    /// one of a drawn delete action, or a delete of a drawn row when no field holds a reference.
    fn delete_referenced(&mut self, rng: &mut Rng) -> Stmt {
        let mut refs: Vec<(usize, Option<usize>, &Field)> = Vec::new();
        for (i, r) in self.s.rels.iter().enumerate() {
            refs.extend(r.header.iter().map(|f| (i, None, f)));
            for (c, ctor) in r.ctors.iter().enumerate() {
                refs.extend(ctor.fields.iter().map(|f| (i, Some(c), f)));
            }
        }
        refs.retain(|(_, _, f)| f.ty.target().is_some());
        let action = *rng.pick(&[
            Action::Retag,
            Action::Clear,
            Action::Restrict,
            Action::Cascade,
        ]);
        let acting: Vec<_> = refs
            .iter()
            .copied()
            .filter(|(i, _, f)| match action {
                Action::Restrict => f.restricts(),
                Action::Cascade => {
                    f.action == action && f.ty.target().is_some_and(|(j, _)| j != *i)
                }
                a => f.action == a,
            })
            .collect();
        let pool = if !acting.is_empty() && rng.below(4) != 0 {
            &acting
        } else {
            &refs
        };
        let Some(&(i, c, f)) = rng.pick_some(pool) else {
            let i = rng.below(self.s.rels.len() as u64) as usize;
            return self.delete(rng, i);
        };
        let (j, scope) = f.ty.target().expect("a reference has a target");
        let from = self.s.rels[i].target(c.map(|c| vec![c]).as_ref());
        let target = &self.s.rels[j];
        let over = match scope {
            Scope::All => target.name.clone(),
            Scope::Ctors(cs) => target.target(Some(cs)),
            Scope::Named => target.target(target.set.as_ref()),
        };
        let mut conds = vec![format!("y.{} == {}", f.name, f.holding_key("x"))];
        if !f.restricts() {
            for (k, c, g) in refs
                .iter()
                .filter(|(_, _, g)| g.restricts() && g.ty.target() == Some((j, scope)))
            {
                let z = format!("z{}", conds.len());
                conds.push(format!(
                    "(not exists {z} in {} where {z}.{} == {})",
                    self.s.rels[*k].target(c.map(|c| vec![c]).as_ref()),
                    g.name,
                    g.holding_key("x")
                ));
            }
        }
        let mut s = Stmt::write(format!(
            "delete x in (from x in {over}, y in {from} where {} select x)",
            conds.join(" and ")
        ));
        s.deletes.push(target.name.clone());
        s
    }

    /// Returns a retag of a row of relation `i` to a drawn constructor, or an update when the
    /// relation is a record or no constructor can be retagged to. A reference to a narrowed
    /// constructor set in the new constructor copies a field of its type from the row's own.
    fn retag(&mut self, rng: &mut Rng, i: usize) -> Stmt {
        let r = &self.s.rels[i];
        let copied = |c: usize, d: usize| {
            c != d
                && r.ctors[d]
                    .fields
                    .iter()
                    .filter(|f| matches!(f.ty, Ty::Ref(..)) && f.ty.narrowed())
                    .all(|f| r.ctors[c].fields.iter().any(|g| g.ty == f.ty))
        };
        let moves: Vec<(usize, usize)> = (0..r.ctors.len())
            .flat_map(|c| (0..r.ctors.len()).map(move |d| (c, d)))
            .filter(|(c, d)| r.sum && r.states[*d] == Stage::Enabled && copied(*c, *d))
            .collect();
        let Some(&(c, d)) = rng.pick_some(&moves) else {
            return self.update(rng, i);
        };
        let narrowed = r.ctors[d].fields.iter().any(|f| f.ty.narrowed());
        let mut fs = Vec::new();
        for f in &r.ctors[d].fields {
            if r.has_default(d, &f.name) && rng.below(2) == 0 {
                continue;
            }
            let own = r.ctors[c].fields.iter().filter(|g| g.ty == f.ty);
            let v = match own.map(|g| format!("x.{}", g.name)).collect::<Vec<_>>() {
                vs if f.ty.narrowed() && !vs.is_empty() => rng.pick(&vs).clone(),
                _ if f.ty.narrowed() => "None".into(),
                _ => self.value(rng, &f.ty),
            };
            fs.push(format!("{}: {v}", f.name));
        }
        let over = if narrowed {
            r.target(Some(&vec![c]))
        } else {
            r.name.clone()
        };
        let mut s = Stmt::write(format!(
            "update x in {over} where key x == {} set {} {{ {} }}",
            self.key(rng, i),
            r.ctors[d].name,
            fs.join(", ")
        ));
        s.marks.push(Mark::Retag);
        s
    }

    fn update(&mut self, rng: &mut Rng, i: usize) -> Stmt {
        let r = &self.s.rels[i];
        let Some(&c) = rng.pick_some(&r.enabled()) else {
            return self.query(rng, i);
        };
        let fields: Vec<&Field> = r
            .plain_header()
            .iter()
            .chain(&r.ctors[c].fields)
            .filter(|f| !f.ty.narrowed())
            .collect();
        let Some(f) = rng.pick_some(&fields) else {
            return self.query(rng, i);
        };
        Stmt::write(format!(
            "update x in {} where key x == {} set {} {{ x with {}: {} }}",
            r.target(Some(&vec![c])),
            self.key(rng, i),
            r.ctors[c].name,
            f.name,
            self.value(rng, &f.ty)
        ))
    }

    /// Returns two writes as one statement.
    fn pair(&mut self, rng: &mut Rng) -> Stmt {
        let one = |d: &mut Self, rng: &mut Rng| {
            let i = rng.below(d.s.rels.len() as u64) as usize;
            match rng.below(4) {
                0 => d.insert(rng, i),
                1 | 2 => d.delete(rng, i),
                _ => d.retag(rng, i),
            }
        };
        let a = one(self, rng);
        let b = one(self, rng);
        if !(a.write && b.write) {
            return a;
        }
        let mut s = Stmt::write(format!("({}, {})", a.text, b.text));
        s.deletes = a.deletes.into_iter().chain(b.deletes).collect();
        s.marks = a.marks.into_iter().chain(b.marks).collect();
        s
    }

    fn query(&mut self, rng: &mut Rng, i: usize) -> Stmt {
        let r = &self.s.rels[i];
        let refs: Vec<(usize, &Field)> = (0..r.ctors.len())
            .flat_map(|c| r.ctors[c].fields.iter().map(move |f| (c, f)))
            .filter(|(_, f)| f.ty.target().is_some())
            .collect();
        match rng.pick_some(&refs) {
            Some((c, f)) if rng.below(2) == 0 => match f.ty {
                Ty::Ref(j, _) => Stmt::query(format!(
                    "from x in {}, y in {} where x.{} == key y select {{ a: key x, b: key y }}",
                    r.target(Some(&vec![*c])),
                    self.s.rels[j].name,
                    f.name
                )),
                _ => Stmt::query(format!(
                    "from x in {} select {{ a: key x, b: x.{} }}",
                    r.target(Some(&vec![*c])),
                    f.name
                )),
            },
            _ if r.sum && r.ctors.len() >= 2 => {
                let cs = match &r.set {
                    Some(set) if rng.below(2) == 0 => set.clone(),
                    _ => r.pair(rng),
                };
                Stmt::query(format!("from x in {} select x", r.target(Some(&cs))))
            }
            _ => Stmt::query(format!("from x in {} select key x", r.name)),
        }
    }
}
