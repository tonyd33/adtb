//! Generated schemas: relations of record and sum types whose fields hold value-level sums,
//! options and references typed by constructor sets, with delete actions, uniques, indexes,
//! defaults, checks and constraints, and the statements run over them, each a function of its
//! seed alone.

mod draw;

use draw::Draw;

use crate::Rng;

/// The value-level sums every schema declares, the second nesting the first.
const SUMS: [&str; 2] = [
    "type V0 = Z | S { v: I64 } | W { t: Text, o: Option I64 }",
    "type V1 = M | N { w: V0 }",
];

/// The names of constructor fields, which the constructors of one sum share.
const NAMES: [&str; 4] = ["a", "b", "n", "u"];

/// The constructor names of every relation-level sum.
const CTORS: [&str; 4] = ["A", "B", "C", "D"];

/// The name of the one named set a relation may declare.
const SET: &str = "Pair";

/// Whether a retag payload may give a field whose literal is a constructor, in place of a
/// default for it.
const SEARCH_RETAG_PAYLOAD_NULLARY_PANICS: bool = false;

/// Whether a reference to its own relation may name a constructor set narrower than the
/// relation.
const SEARCH_KEY_VIOLATION_NAMED_AS_REFERENCE: bool = false;

/// The constructor sets a reference, unique, index or check names within one relation.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    All,
    Ctors(Vec<usize>),
    Named,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Ty {
    Int,
    Text,
    Bool,
    OptInt,
    Sum(usize),
    Ref(usize, Scope),
    OptRef(usize, Scope),
}

impl Ty {
    fn target(&self) -> Option<(usize, &Scope)> {
        match self {
            Ty::Ref(j, s) | Ty::OptRef(j, s) => Some((*j, s)),
            _ => None,
        }
    }

    /// Returns whether a retag payload can give a value of this type as a literal.
    fn literal(&self) -> bool {
        !matches!(self, Ty::Ref(..))
    }

    /// Returns whether the type is a reference to a constructor set narrower than its relation,
    /// whose values a statement takes from a binder over that set.
    fn narrowed(&self) -> bool {
        self.target().is_some_and(|(_, s)| *s != Scope::All)
    }

    /// Returns whether a unique, an index or a check may name a field of this type.
    fn keyed(&self) -> bool {
        matches!(self, Ty::Int | Ty::Text | Ty::Ref(..))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Unstated,
    Restrict,
    Cascade,
    Clear,
    Retag,
}

#[derive(Clone, Debug)]
struct Field {
    name: String,
    ty: Ty,
    action: Action,
}

impl Field {
    fn new(name: &str, ty: Ty) -> Field {
        Field {
            name: name.to_owned(),
            ty,
            action: Action::Unstated,
        }
    }

    /// Returns whether the field is a reference whose delete action is `restrict`, stated or not.
    fn restricts(&self) -> bool {
        self.ty.target().is_some() && matches!(self.action, Action::Restrict | Action::Unstated)
    }

    /// Returns the expression of the field's value holding the key of binder `x`.
    fn holding_key(&self, x: &str) -> String {
        match self.ty {
            Ty::OptRef(..) => format!("Some {{ value: key {x} }}"),
            _ => format!("key {x}"),
        }
    }
}

#[derive(Clone, Debug)]
struct Ctor {
    name: String,
    fields: Vec<Field>,
}

impl Ctor {
    fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// A unique, an index or a check: its field and the constructors it ranges over, every one
/// when `None`.
#[derive(Clone, Debug)]
struct Clause {
    field: String,
    ctors: Option<Vec<usize>>,
}

#[derive(Clone, Debug)]
struct Rel {
    name: String,
    ty: String,
    sum: bool,
    /// The header fields forming the key, first in `header`; none for a generated `id`.
    keys: usize,
    header: Vec<Field>,
    ctors: Vec<Ctor>,
    /// The constructor every `retag` action of the type names.
    retag: Option<usize>,
    /// The constructors of the named set [`SET`], when the relation declares it.
    set: Option<Vec<usize>>,
    uniques: Vec<Clause>,
    indexes: Vec<Clause>,
    checks: Vec<Clause>,
    /// Each default: its constructor, field and literal.
    defaults: Vec<(usize, String, String)>,
}

impl Rel {
    fn shape(rng: &mut Rng, i: usize) -> Rel {
        let sum = rng.below(4) != 0;
        let ty = format!("T{i}");
        let ctors = if sum {
            CTORS[..2 + rng.below(3) as usize]
                .iter()
                .map(|c| Ctor {
                    name: (*c).to_owned(),
                    fields: Vec::new(),
                })
                .collect()
        } else {
            vec![Ctor {
                name: ty.clone(),
                fields: Vec::new(),
            }]
        };
        let mut rel = Rel {
            name: format!("r{i}"),
            ty,
            sum,
            keys: if rng.below(5) == 0 { 2 } else { 0 },
            header: Vec::new(),
            ctors,
            retag: None,
            set: None,
            uniques: Vec::new(),
            indexes: Vec::new(),
            checks: Vec::new(),
            defaults: Vec::new(),
        };
        if rel.ctors.len() >= 3 && rng.below(2) == 0 {
            rel.set = Some(rel.pair(rng));
        }
        rel
    }

    /// Returns two constructors of the relation, ascending.
    fn pair(&self, rng: &mut Rng) -> Vec<usize> {
        let n = self.ctors.len() as u64;
        let a = rng.below(n) as usize;
        let b = (a + 1 + rng.below(n - 1) as usize) % n as usize;
        vec![a.min(b), a.max(b)]
    }

    fn ctor_set(&self, cs: &[usize]) -> String {
        let names: Vec<&str> = cs.iter().map(|c| self.ctors[*c].name.as_str()).collect();
        match names.as_slice() {
            [one] => (*one).to_owned(),
            _ => format!("({})", names.join(" | ")),
        }
    }

    /// Returns how a clause of the relation names `ctors` after `in`, or `None` for every one.
    fn scope(&self, ctors: Option<&Vec<usize>>) -> Option<String> {
        let cs = ctors?;
        Some(if self.set.as_ref() == Some(cs) {
            SET.to_owned()
        } else {
            self.ctor_set(cs)
        })
    }

    /// Returns the relation expression of constructors `ctors`, or of the whole relation.
    fn target(&self, ctors: Option<&Vec<usize>>) -> String {
        match self.scope(ctors) {
            Some(s) if self.sum => format!("{}.{s}", self.name),
            _ => self.name.clone(),
        }
    }

    /// Returns the header fields that are not key fields.
    fn plain_header(&self) -> &[Field] {
        &self.header[self.keys..]
    }

    /// Returns the number of fields holding a reference an insert must give.
    fn refs(&self) -> usize {
        self.header
            .iter()
            .chain(self.ctors.iter().flat_map(|c| &c.fields))
            .filter(|f| matches!(f.ty, Ty::Ref(..)))
            .count()
    }

    fn has_default(&self, c: usize, f: &str) -> bool {
        self.defaults.iter().any(|(d, g, _)| *d == c && g == f)
    }

    /// Returns the constructors holding a field named `f` of type `ty`.
    fn holding(&self, f: &str, ty: &Ty) -> Vec<usize> {
        (0..self.ctors.len())
            .filter(|c| self.ctors[*c].field(f).is_some_and(|g| g.ty == *ty))
            .collect()
    }
}

/// A schema of relations over the sums [`SUMS`], with its constraints.
#[derive(Clone, Debug)]
struct Schema {
    rels: Vec<Rel>,
    /// Each constraint: its relation, constructor and integer field, whose rows may not hold 3.
    constraints: Vec<(usize, usize, String)>,
}

impl Schema {
    fn draw(rng: &mut Rng) -> Schema {
        let n = 2 + rng.below(3) as usize;
        let mut s = Schema {
            rels: (0..n).map(|i| Rel::shape(rng, i)).collect(),
            constraints: Vec::new(),
        };
        for i in 0..n {
            s.fields(rng, i);
        }
        for i in 0..n {
            s.actions(rng, i);
        }
        for i in 0..n {
            s.clauses(rng, i);
        }
        s
    }

    /// Returns a field type of relation `i`, a reference to relation `i` itself over every
    /// constructor unless [`SEARCH_KEY_VIOLATION_NAMED_AS_REFERENCE`].
    fn ty(&self, rng: &mut Rng, i: usize) -> Ty {
        match rng.below(10) {
            0 | 1 => Ty::Int,
            2 => Ty::Text,
            3 => Ty::Bool,
            4 => Ty::OptInt,
            5 => Ty::Sum(rng.below(2) as usize),
            k => {
                let j = rng.below(self.rels.len() as u64) as usize;
                let scope = match self.ref_scope(rng, j) {
                    _ if j == i && !SEARCH_KEY_VIOLATION_NAMED_AS_REFERENCE => Scope::All,
                    s => s,
                };
                if k < 8 {
                    Ty::Ref(j, scope)
                } else {
                    Ty::OptRef(j, scope)
                }
            }
        }
    }

    fn ref_scope(&self, rng: &mut Rng, j: usize) -> Scope {
        let r = &self.rels[j];
        if !r.sum {
            return Scope::All;
        }
        match rng.below(4) {
            0 => Scope::All,
            1 => Scope::Ctors(vec![rng.below(r.ctors.len() as u64) as usize]),
            2 => Scope::Ctors(r.pair(rng)),
            _ if r.set.is_some() => Scope::Named,
            _ => Scope::All,
        }
    }

    /// Draw the key, header and constructor fields of relation `i`.
    fn fields(&mut self, rng: &mut Rng, i: usize) {
        let mut header = Vec::new();
        if self.rels[i].keys > 0 {
            header.push(Field::new("k1", Ty::Int));
            let generated: Vec<usize> = (0..self.rels.len())
                .filter(|j| self.rels[*j].keys == 0)
                .collect();
            let k2 = match (rng.pick_some(&generated), rng.below(3)) {
                (Some(j), 0) => Ty::Ref(*j, Scope::All),
                (_, k) => Ty::Sum(k as usize % 2),
            };
            let mut k2 = Field::new("k2", k2);
            if k2.ty.target().is_some() && rng.below(2) == 0 {
                k2.action = Action::Cascade;
            }
            header.push(k2);
        }
        for h in 0..rng.below(2) {
            header.push(Field::new(&format!("h{h}"), self.ty(rng, i)));
        }
        let sum = self.rels[i].sum;
        let mut own: Vec<Vec<Field>> = Vec::new();
        for _ in 0..self.rels[i].ctors.len() {
            let wanted = if sum { rng.below(4) } else { 1 + rng.below(3) };
            let mut fields: Vec<Field> = Vec::new();
            for _ in 0..wanted {
                let name = *rng.pick(&NAMES);
                if fields.iter().any(|f| f.name == name) {
                    continue;
                }
                let shared = own
                    .iter()
                    .find_map(|fs| fs.iter().find(|f| f.name == name).map(|f| f.ty.clone()));
                let ty = match shared {
                    Some(ty) if rng.below(3) != 0 => ty,
                    _ => self.ty(rng, i),
                };
                fields.push(Field::new(name, ty));
            }
            own.push(fields);
        }
        if sum && rng.below(2) == 0 {
            let j = rng.below(self.rels.len() as u64) as usize;
            let pair = self.rels[i].pair(rng);
            plant(&mut own, pair[0], pair[1], j);
        }
        if own.len() > 2 && rng.below(4) != 0 {
            let pair = self.rels[i].pair(rng);
            share(&mut own, pair[0], pair[1]);
        }
        let rel = &mut self.rels[i];
        for (c, fields) in rel.ctors.iter_mut().zip(own) {
            c.fields = fields;
        }
        rel.header = header;
    }

    /// Draw the delete action of each reference field of relation `i`.
    fn actions(&mut self, rng: &mut Rng, i: usize) {
        let rel = &mut self.rels[i];
        let keys = rel.keys;
        for f in rel.header.iter_mut().skip(keys) {
            f.action = plain_action(rng, &f.ty);
        }
        for c in 0..rel.ctors.len() {
            for k in 0..rel.ctors[c].fields.len() {
                let ty = rel.ctors[c].fields[k].ty.clone();
                if ty.target().is_none() {
                    continue;
                }
                let action = match retag_to(rel, c, &rel.ctors[c].fields[k].name) {
                    Some(d) if rng.below(3) != 0 => {
                        if rel.retag.is_none() && !SEARCH_RETAG_PAYLOAD_NULLARY_PANICS {
                            default_sums(rel, d);
                        }
                        rel.retag = Some(d);
                        Action::Retag
                    }
                    _ => plain_action(rng, &ty),
                };
                rel.ctors[c].fields[k].action = action;
                if action == Action::Retag {
                    clear_beside(rel, c, &ty);
                }
            }
        }
    }

    /// Draw the uniques, indexes, checks, defaults and constraints of relation `i`.
    fn clauses(&mut self, rng: &mut Rng, i: usize) {
        let mut constraints = Vec::new();
        let rel = &mut self.rels[i];
        for f in rel.header[rel.keys..].iter().filter(|f| f.ty.keyed()) {
            let whole = Clause {
                field: f.name.clone(),
                ctors: None,
            };
            match rng.below(4) {
                0 => rel.uniques.push(whole),
                1 => rel.indexes.push(whole),
                2 if f.ty == Ty::Int => rel.checks.push(whole),
                _ => {}
            }
        }
        let mut fields: Vec<(String, Ty)> = Vec::new();
        for c in &rel.ctors {
            for f in &c.fields {
                if f.ty.keyed() && !fields.iter().any(|(n, t)| *n == f.name && *t == f.ty) {
                    fields.push((f.name.clone(), f.ty.clone()));
                }
            }
        }
        let set_wide = |rel: &Rel, h: &[usize]| h.len() > 1 && h.len() < rel.ctors.len();
        let shared: Vec<&(String, Ty)> = fields
            .iter()
            .filter(|(n, t)| matches!(t, Ty::Int | Ty::Text) && set_wide(rel, &rel.holding(n, t)))
            .collect();
        if let Some((name, ty)) = rng.pick_some(&shared).copied()
            && rng.below(4) != 0
        {
            rel.uniques.push(Clause {
                field: name.clone(),
                ctors: Some(rel.holding(name, ty)),
            });
        }
        for (name, ty) in fields {
            let mut holding = rel.holding(&name, &ty);
            if rel.uniques.iter().any(|u| u.field == name) {
                continue;
            }
            if rel.sum && holding.len() == rel.ctors.len() {
                holding.remove(rng.below(holding.len() as u64) as usize);
            }
            if holding.is_empty() {
                continue;
            }
            if holding.len() > 1 && rng.below(4) == 0 {
                holding = vec![*rng.pick(&holding)];
            }
            let clause = Clause {
                field: name.clone(),
                ctors: rel.sum.then_some(holding.clone()),
            };
            match rng.below(5) {
                0 | 1 => rel.uniques.push(clause),
                _ if holding.len() > 1 && rng.below(2) == 0 => rel.uniques.push(clause),
                2 => rel.indexes.push(clause),
                3 if ty == Ty::Int => rel.checks.push(clause),
                4 if ty == Ty::Int => constraints.push((i, holding[0], name)),
                _ => {}
            }
        }
        for c in 0..rel.ctors.len() {
            for f in &rel.ctors[c].fields {
                if f.ty.literal() && !rel.has_default(c, &f.name) && rng.below(4) == 0 {
                    rel.defaults
                        .push((c, f.name.clone(), literal(&f.ty, rng.below(2) == 0)));
                }
            }
        }
        self.constraints.extend(constraints);
    }

    fn ty_name(&self, ty: &Ty) -> String {
        match ty {
            Ty::Int => "I64".into(),
            Ty::Text => "Text".into(),
            Ty::Bool => "Bool".into(),
            Ty::OptInt => "Option I64".into(),
            Ty::Sum(k) => format!("V{k}"),
            Ty::Ref(j, s) => self.key_ty(*j, s),
            Ty::OptRef(j, s) => format!("Option ({})", self.key_ty(*j, s)),
        }
    }

    fn key_ty(&self, j: usize, scope: &Scope) -> String {
        let r = &self.rels[j];
        match scope {
            Scope::All => format!("Key {}", r.name),
            Scope::Ctors(cs) => format!("Key {}.{}", r.name, r.ctor_set(cs)),
            Scope::Named => format!("Key {}.{SET}", r.name),
        }
    }

    fn field_decl(&self, rel: &Rel, f: &Field) -> String {
        let action = match f.action {
            Action::Unstated => String::new(),
            Action::Restrict => " on delete restrict".into(),
            Action::Cascade => " on delete cascade".into(),
            Action::Clear => " on delete clear".into(),
            Action::Retag => {
                let d = rel.retag.expect("a retag action names its constructor");
                format!(" on delete retag {}", payload(rel, d))
            }
        };
        format!("{}: {}{action}", f.name, self.ty_name(&f.ty))
    }

    fn type_decl(&self, rel: &Rel) -> String {
        let fields = |id: bool, fs: &mut dyn Iterator<Item = &Field>| {
            let mut out: Vec<String> = Vec::new();
            if id {
                out.push(format!("id: Key {}", rel.name));
            }
            out.extend(fs.map(|f| self.field_decl(rel, f)));
            format!("{{ {} }}", out.join(", "))
        };
        let id = rel.keys == 0;
        if !rel.sum {
            let mut all = rel.header.iter().chain(&rel.ctors[0].fields);
            return format!("type {} = {}", rel.ty, fields(id, &mut all));
        }
        let mut out = format!("type {} {}", rel.ty, fields(id, &mut rel.header.iter()));
        for (k, c) in rel.ctors.iter().enumerate() {
            out.push_str(if k == 0 { "\n  = " } else { "\n  | " });
            out.push_str(&c.name);
            if !c.fields.is_empty() {
                out.push(' ');
                out.push_str(&fields(false, &mut c.fields.iter()));
            }
        }
        out
    }

    fn relation_decl(rel: &Rel) -> String {
        let mut out = if rel.keys == 0 {
            format!(
                "relation {} : {} key id generate sequence",
                rel.name, rel.ty
            )
        } else {
            let ks: Vec<&str> = rel.header[..rel.keys]
                .iter()
                .map(|f| f.name.as_str())
                .collect();
            format!("relation {} : {} key ({})", rel.name, rel.ty, ks.join(", "))
        };
        let clause = |verb: &str, c: &Clause| match c.ctors.as_ref().map(|cs| rel.ctor_set(cs)) {
            Some(s) => format!("\n  {verb} ({}) in {s}", c.field),
            None => format!("\n  {verb} ({})", c.field),
        };
        for u in &rel.uniques {
            out.push_str(&clause("unique", u));
        }
        for x in &rel.indexes {
            out.push_str(&clause("index", x));
        }
        for (c, f, v) in &rel.defaults {
            if rel.sum {
                out.push_str(&format!("\n  default {}.{f} = {v}", rel.ctors[*c].name));
            } else {
                out.push_str(&format!("\n  default {f} = {v}"));
            }
        }
        out
    }

    /// Returns the declarations of module `app`, without its `module` line.
    fn decls(&self) -> String {
        let mut out: Vec<String> = SUMS.iter().map(|s| (*s).to_owned()).collect();
        for rel in &self.rels {
            out.push(self.type_decl(rel));
            out.push(Self::relation_decl(rel));
            if let Some(cs) = &rel.set {
                out.push(format!(
                    "set {SET} of {} = {}",
                    rel.name,
                    rel.ctor_set(cs).trim_matches(['(', ')'])
                ));
            }
        }
        for (i, rel) in self.rels.iter().enumerate() {
            for (k, c) in rel.checks.iter().enumerate() {
                out.push(format!(
                    "check chk{i}x{k} on {} ({} >= (-2))",
                    rel.target(c.ctors.as_ref()),
                    c.field
                ));
            }
        }
        for (k, (i, c, f)) in self.constraints.iter().enumerate() {
            let rel = &self.rels[*i];
            out.push(format!(
                "constraint con{k} = from x in {} where x.{f} == 3 select x",
                rel.target(Some(&vec![*c]))
            ));
        }
        let mut text = out.join("\n");
        text.push('\n');
        text
    }
}

/// Give constructor `c` of `own` a reference to relation `j` when it has none, and `d` an
/// integer field of a name `c` lacks when it has no literal field of such a name.
fn plant(own: &mut [Vec<Field>], c: usize, d: usize, j: usize) {
    let free = |fs: &[Field], name: &str| fs.iter().all(|f| f.name != name);
    if !own[c].iter().any(|f| matches!(f.ty, Ty::Ref(..))) {
        let Some(name) = NAMES.iter().find(|n| free(&own[c], n) && free(&own[d], n)) else {
            return;
        };
        own[c].push(Field::new(name, Ty::Ref(j, Scope::All)));
    }
    let named = own[d]
        .iter()
        .any(|f| free(&own[c], &f.name) && f.ty.literal());
    if !named && let Some(name) = NAMES.iter().find(|n| free(&own[c], n) && free(&own[d], n)) {
        own[d].push(Field::new(name, Ty::Int));
    }
}

/// Give constructor `d` a copy of a field of `c` of a type a unique may name, of a name `d` lacks.
fn share(own: &mut [Vec<Field>], c: usize, d: usize) {
    let lacking = |f: &&Field| f.ty.keyed() && own[d].iter().all(|g| g.name != f.name);
    if let Some(f) = own[c].iter().find(lacking).cloned() {
        own[d].push(f);
    }
}

/// Returns a delete action for a reference field of type `ty` that is not `retag`.
fn plain_action(rng: &mut Rng, ty: &Ty) -> Action {
    match (ty, rng.below(5)) {
        (Ty::Ref(..) | Ty::OptRef(..), 0) => Action::Restrict,
        (Ty::Ref(..) | Ty::OptRef(..), 1 | 2) => Action::Cascade,
        (Ty::OptRef(..), 3 | 4) => Action::Clear,
        _ => Action::Unstated,
    }
}

/// Returns the constructor a `retag` action on field `f` of constructor `c` may name: the one
/// the type's actions already name, else one drawn among those lacking `f` whose fields a
/// payload of literals can give.
fn retag_to(rel: &Rel, c: usize, f: &str) -> Option<usize> {
    let fits = |d: usize| {
        d != c
            && rel.ctors[d].field(f).is_none()
            && rel.ctors[d].fields.iter().all(|g| g.ty.literal())
    };
    if !rel.sum {
        return None;
    }
    match rel.retag {
        Some(d) => fits(d).then_some(d),
        None => (0..rel.ctors.len()).find(|d| fits(*d)),
    }
}

/// Give each field of constructor `d` whose literal is a constructor a default.
fn default_sums(rel: &mut Rel, d: usize) {
    for f in &rel.ctors[d].fields {
        if matches!(f.ty, Ty::Sum(_) | Ty::OptInt | Ty::OptRef(..)) {
            rel.defaults
                .push((d, f.name.clone(), literal(&f.ty, false)));
        }
    }
}

/// Give constructor `c`, whose reference field of type `ty` retags to the type's `retag`
/// constructor, a `clear` field of the target of `ty` named as an own field of that
/// constructor.
fn clear_beside(rel: &mut Rel, c: usize, ty: &Ty) {
    let d = rel.retag.expect("a retag action names its constructor");
    let Some((target, _)) = ty.target() else {
        return;
    };
    let Some(name) = rel.ctors[d]
        .fields
        .iter()
        .find(|g| rel.ctors[c].field(&g.name).is_none())
        .map(|g| g.name.clone())
    else {
        return;
    };
    rel.ctors[c].fields.push(Field {
        name,
        ty: Ty::OptRef(target, Scope::All),
        action: Action::Clear,
    });
}

/// Returns the payload of a retag to constructor `d`: a literal for each of its fields without
/// a default.
fn payload(rel: &Rel, d: usize) -> String {
    let fs: Vec<String> = rel.ctors[d]
        .fields
        .iter()
        .filter(|f| !rel.has_default(d, &f.name))
        .map(|f| format!("{}: {}", f.name, literal(&f.ty, false)))
        .collect();
    format!("{} {{ {} }}", rel.ctors[d].name, fs.join(", "))
}

/// Returns a literal of type `ty`, a reference only as `None`; `other` picks a second one.
fn literal(ty: &Ty, other: bool) -> String {
    match (ty, other) {
        (Ty::Int, false) => "5".into(),
        (Ty::Int, true) => "(-3)".into(),
        (Ty::Text, false) => r#""p""#.into(),
        (Ty::Text, true) => r#""a""#.into(),
        (Ty::Bool, o) => o.to_string(),
        (Ty::Sum(0), _) => "Z".into(),
        (Ty::Sum(_), _) => "M".into(),
        (Ty::OptInt | Ty::OptRef(..), _) => "None".into(),
        (Ty::Ref(..), _) => unreachable!("a payload gives no reference"),
    }
}

/// A shape a statement reaches once it commits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// An update to another constructor.
    Retag,
    /// An insert of a row of a sum holding a reference to a constructor set of another sum.
    CrossSum,
    /// An insert of a row whose key holds a value-level sum.
    SumKey,
}

/// A statement of a case: a write, or a query.
#[derive(Clone, Debug)]
pub struct Stmt {
    text: String,
    write: bool,
    deletes: Vec<String>,
    marks: Vec<Mark>,
}

impl Stmt {
    fn query(text: String) -> Stmt {
        Stmt {
            text,
            write: false,
            deletes: Vec::new(),
            marks: Vec::new(),
        }
    }

    fn write(text: String) -> Stmt {
        Stmt {
            write: true,
            ..Stmt::query(text)
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_write(&self) -> bool {
        self.write
    }

    /// Returns the relations the statement deletes from by its own writes.
    pub fn deletes(&self) -> &[String] {
        &self.deletes
    }

    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }
}

/// A generated schema and the statements run over it in order.
#[derive(Clone, Debug)]
pub struct Case {
    schema: Schema,
    stmts: Vec<Stmt>,
}

impl Case {
    /// Returns the declarations of module `app`, without its `module` line.
    pub fn decls(&self) -> String {
        self.schema.decls()
    }

    pub fn stmts(&self) -> &[Stmt] {
        &self.stmts
    }

    /// Returns each relation's name and its key fields.
    pub fn relations(&self) -> Vec<(String, Vec<String>)> {
        let key = |r: &Rel| match r.keys {
            0 => vec!["id".to_owned()],
            k => r.header[..k].iter().map(|f| f.name.clone()).collect(),
        };
        self.schema
            .rels
            .iter()
            .map(|r| (r.name.clone(), key(r)))
            .collect()
    }

    /// Returns whether constructor `from` of relation `rel` holds a `clear` field whose name
    /// constructor `to` has.
    pub fn shares_clear(&self, rel: usize, from: u32, to: u32) -> bool {
        let r = &self.schema.rels[rel];
        let (Some(c), Some(d)) = (r.ctors.get(from as usize), r.ctors.get(to as usize)) else {
            return false;
        };
        c.fields
            .iter()
            .any(|f| f.action == Action::Clear && d.field(&f.name).is_some())
    }

    /// Returns whether `constraint` is the canonical name of a reference field whose delete
    /// action is `restrict`, stated or not.
    pub fn restricts(&self, constraint: &str) -> bool {
        let restricting = |f: &Field| f.restricts();
        self.schema.rels.iter().any(|r| {
            let header = r.header.iter().filter(|f| restricting(f)).map(|f| {
                if r.sum {
                    format!("app::{}.{}", r.name, f.name)
                } else {
                    format!("app::{}.{}.{}", r.name, r.ty, f.name)
                }
            });
            let own = r.ctors.iter().flat_map(|c| {
                c.fields
                    .iter()
                    .filter(|f| restricting(f))
                    .map(|f| format!("app::{}.{}.{}", r.name, c.name, f.name))
            });
            header.chain(own).any(|n| n == constraint)
        })
    }

    /// Returns the declarations and each statement, a statement on a line.
    pub fn program(&self) -> String {
        let mut out = self.decls();
        for s in &self.stmts {
            out.push_str(&s.text);
            out.push('\n');
        }
        out
    }

    /// Returns the case holding its first `n` statements.
    pub fn first(&self, n: usize) -> Case {
        Case {
            stmts: self.stmts[..n.min(self.stmts.len())].to_vec(),
            ..self.clone()
        }
    }

    /// Returns the cases one step smaller: each without a run of statements, the longest runs
    /// first, then each without one relation no other references and the statements naming it,
    /// each without one field and the clauses naming it, and each without one unique, index,
    /// check or constraint.
    pub fn smaller(&self) -> Vec<Case> {
        let mut out = Vec::new();
        let len = self.stmts.len();
        let mut size = len.div_ceil(2);
        while size > 0 {
            for start in (0..len).step_by(size) {
                let mut c = self.clone();
                c.stmts.drain(start..(start + size).min(len));
                out.push(c);
            }
            size /= 2;
        }
        out.extend((0..self.schema.rels.len()).filter_map(|k| self.without_rel(k)));
        for (i, rel) in self.schema.rels.iter().enumerate() {
            let mut names: Vec<&str> = rel
                .plain_header()
                .iter()
                .chain(rel.ctors.iter().flat_map(|c| &c.fields))
                .map(|f| f.name.as_str())
                .collect();
            names.sort_unstable();
            names.dedup();
            out.extend(names.into_iter().map(|f| self.without_field(i, f)));
        }
        for (i, rel) in self.schema.rels.iter().enumerate() {
            let drops: [(usize, Clauses); 3] = [
                (rel.uniques.len(), |r| &mut r.uniques),
                (rel.indexes.len(), |r| &mut r.indexes),
                (rel.checks.len(), |r| &mut r.checks),
            ];
            for (n, clauses) in drops {
                for k in 0..n {
                    let mut c = self.clone();
                    clauses(&mut c.schema.rels[i]).remove(k);
                    out.push(c);
                }
            }
        }
        for k in 0..self.schema.constraints.len() {
            let mut c = self.clone();
            c.schema.constraints.remove(k);
            out.push(c);
        }
        out
    }

    /// Returns the case without relation `k` and the statements naming it, or `None` when
    /// another relation references it.
    fn without_rel(&self, k: usize) -> Option<Case> {
        let rels = &self.schema.rels;
        let fields = |r: &Rel| -> Vec<Ty> {
            r.header
                .iter()
                .chain(r.ctors.iter().flat_map(|c| &c.fields))
                .map(|f| f.ty.clone())
                .collect()
        };
        let referenced = rels.iter().enumerate().any(|(i, r)| {
            i != k
                && fields(r)
                    .iter()
                    .any(|t| t.target().is_some_and(|(j, _)| j == k))
        });
        if referenced {
            return None;
        }
        let mut c = self.clone();
        let name = c.schema.rels.remove(k).name;
        let lower = |j: &mut usize| *j -= usize::from(*j > k);
        for r in &mut c.schema.rels {
            for f in r
                .header
                .iter_mut()
                .chain(r.ctors.iter_mut().flat_map(|c| &mut c.fields))
            {
                if let Ty::Ref(j, _) | Ty::OptRef(j, _) = &mut f.ty {
                    lower(j);
                }
            }
        }
        c.schema.constraints.retain(|(i, _, _)| *i != k);
        for (i, _, _) in &mut c.schema.constraints {
            lower(i);
        }
        c.stmts.retain(|s| !mentions(&s.text, &name));
        Some(c)
    }

    /// Returns the case without field `f` of relation `i`, the clauses naming it, and the field
    /// in each row a statement naming the relation gives.
    fn without_field(&self, i: usize, f: &str) -> Case {
        let mut c = self.clone();
        let rel = &mut c.schema.rels[i];
        let keys = rel.keys;
        let mut k = 0;
        rel.header.retain(|g| {
            k += 1;
            k <= keys || g.name != f
        });
        for ctor in &mut rel.ctors {
            ctor.fields.retain(|g| g.name != f);
        }
        for clauses in [&mut rel.uniques, &mut rel.indexes, &mut rel.checks] {
            clauses.retain(|u| u.field != f);
        }
        rel.defaults.retain(|(_, g, _)| g != f);
        let name = rel.name.clone();
        c.schema.constraints.retain(|(j, _, g)| *j != i || g != f);
        for s in &mut c.stmts {
            if mentions(&s.text, &name) {
                s.text = without_entry(&s.text, f);
            }
        }
        c
    }
}

/// Returns whether `text` names relation `name`.
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// Returns `text` without each record entry `f: v`.
fn without_entry(text: &str, f: &str) -> String {
    let mut out = text.to_owned();
    for open in ["{ ", ", "] {
        let pat = format!("{open}{f}: ");
        while let Some(at) = out.find(&pat) {
            let start = at + pat.len();
            let mut depth = 0i32;
            let mut quoted = false;
            let mut end = out.len();
            for (k, ch) in out[start..].char_indices() {
                match ch {
                    '"' => quoted = !quoted,
                    _ if quoted => {}
                    '(' | '{' | '[' => depth += 1,
                    ')' | ']' | '}' if depth > 0 => depth -= 1,
                    ',' | '}' | ')' | ']' => {
                        end = start + k;
                        break;
                    }
                    _ => {}
                }
            }
            let tail = &out[end..];
            let (from, to) = match (open, tail.starts_with(", ")) {
                ("{ ", true) => (at + 2, end + 2),
                ("{ ", false) => (at + 2, end),
                _ => (at, end),
            };
            out.replace_range(from..to, "");
        }
    }
    out
}

/// The clauses of one kind of a relation.
type Clauses = fn(&mut Rel) -> &mut Vec<Clause>;

/// Returns the case of `seed`: a drawn schema and `n` statements over it, the first inserting
/// into each relation in turn.
pub fn case(seed: u64, n: usize) -> Case {
    let mut rng = Rng::new(seed);
    let schema = Schema::draw(&mut rng);
    let mut d = Draw::new(&schema);
    let mut order: Vec<usize> = (0..schema.rels.len()).collect();
    order.sort_by_key(|i| schema.rels[*i].refs());
    let stmts = (0..n)
        .map(|k| match order.get(k % (order.len() + 1)) {
            Some(i) if k < 3 * order.len() => d.insert(&mut rng, *i),
            _ => d.stmt(&mut rng),
        })
        .collect();
    Case { schema, stmts }
}
