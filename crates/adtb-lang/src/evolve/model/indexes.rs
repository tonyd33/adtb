use super::print::Printed;
use super::*;
use crate::evolve::Clause;

/// The uniques and indexes of a relation variable, each found by the marker of its clause.
#[derive(Clone, Debug, Default)]
pub(super) struct Indexes(Vec<Held>);

/// The place of a unique or index among its relation variable's clauses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct IndexRef(Key);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Id(u64),
    /// An image an operator made, given its id by [`RelM::identify`] in clause order.
    Pending(u32),
}

/// A unique or index.
#[derive(Clone, Debug)]
pub(super) struct Held {
    key: Key,
    kind: IndexKind,
    scope: ScopeIds,
    elems: Vec<Elem>,
    /// The ids of the clauses this one is the image of, while it has no id of its own.
    was: Vec<u64>,
}

/// The `in` scope of a unique or index.
#[derive(Clone, Debug)]
enum ScopeIds {
    All,
    /// Constructors in written order; `true` when written in parentheses.
    Ctors(Vec<Named>, bool),
    Set(Named),
    /// The parts as written, `true` when in parentheses, for any scope that is neither
    /// constructors alone nor one set alone unparenthesised: a part no build has resolved, a
    /// part naming dropped objects, a name holding two ids, a set beside other parts, or a set
    /// in parentheses.
    Written(Vec<Term>, bool),
}

/// A constructor or named set of a scope, with the name it prints once the model holds no
/// object of its id.
#[derive(Clone, Debug)]
struct Named {
    id: u64,
    written: String,
}

/// An element of a unique or index.
#[derive(Clone, Debug)]
pub(super) struct Elem(Term);

/// A unique or index an operator made, before its relation variable holds it.
#[derive(Clone)]
pub(super) struct NewIndex {
    id: Option<u64>,
    kind: IndexKind,
    scope: ScopeIds,
    elems: Vec<Elem>,
    was: Vec<u64>,
}

impl ScopeIds {
    /// Returns the scope `parts` write, `None` for every constructor.
    fn of(parts: Option<Vec<Term>>, listed: bool) -> ScopeIds {
        let Some(parts) = parts else {
            return ScopeIds::All;
        };
        let named: Option<Vec<(Obj, Named)>> = parts
            .iter()
            .map(|p| {
                let (o, w) = p.as_name()?;
                Some((
                    o,
                    Named {
                        id: o.1,
                        written: w.to_string(),
                    },
                ))
            })
            .collect();
        match named {
            Some(ns) if ns.iter().all(|(o, _)| o.0 == Space::Shared) => {
                ScopeIds::Ctors(ns.into_iter().map(|(_, n)| n).collect(), listed)
            }
            Some(mut ns) if !listed && ns.len() == 1 && ns[0].0.0 == Space::Set => {
                ScopeIds::Set(ns.remove(0).1)
            }
            _ => ScopeIds::Written(parts, listed),
        }
    }

    /// Returns each part as a name, with whether the parts are written in parentheses; `None`
    /// for every constructor.
    fn terms(&self) -> Option<(Vec<Term>, bool)> {
        let name = |space, n: &Named| Term::name((space, n.id), &n.written);
        match self {
            ScopeIds::All => None,
            ScopeIds::Ctors(ns, listed) => {
                Some((ns.iter().map(|n| name(Space::Shared, n)).collect(), *listed))
            }
            ScopeIds::Set(n) => Some((vec![name(Space::Set, n)], false)),
            ScopeIds::Written(ts, listed) => Some((ts.clone(), *listed)),
        }
    }

    /// Returns the first object each part names, `None` for a part naming none.
    fn firsts(&self) -> Vec<Option<Obj>> {
        match self {
            ScopeIds::All => Vec::new(),
            ScopeIds::Ctors(ns, _) => ns.iter().map(|n| Some((Space::Shared, n.id))).collect(),
            ScopeIds::Set(n) => vec![Some((Space::Set, n.id))],
            ScopeIds::Written(ts, _) => ts.iter().map(|t| t.objects().next()).collect(),
        }
    }
}

impl Elem {
    pub(super) fn term(&self) -> &Term {
        &self.0
    }

    /// Returns the objects the element names.
    pub(super) fn objects(&self) -> impl Iterator<Item = Obj> + '_ {
        self.0.objects()
    }

    /// Returns the element as `model` spells it.
    pub(super) fn print(&self, model: &Model) -> String {
        self.0.print(model)
    }
}

impl IndexRef {
    /// Returns the id of the unique or index, `None` while it is an image without one.
    pub(super) fn id(&self) -> Option<u64> {
        match self.0 {
            Key::Id(x) => Some(x),
            Key::Pending(_) => None,
        }
    }
}

impl NewIndex {
    /// Returns a unique or index of kind `kind` over `elems` in the scope `parts` write, every
    /// constructor when `None`, as the image of the clauses of ids `was`.
    pub(super) fn new(
        kind: IndexKind,
        elems: Vec<Term>,
        parts: Option<Vec<Term>>,
        was: Vec<u64>,
    ) -> NewIndex {
        let listed = parts.as_ref().is_some_and(|p| p.len() > 1);
        NewIndex {
            id: None,
            kind,
            scope: ScopeIds::of(parts, listed),
            elems: elems.into_iter().map(Elem).collect(),
            was,
        }
    }

    /// Returns the unique or index with id `id` in place of an image's.
    pub(super) fn identified(self, id: u64) -> NewIndex {
        NewIndex {
            id: Some(id),
            was: Vec::new(),
            ..self
        }
    }

    /// Hold it also as the image of the clauses of ids `was`.
    pub(super) fn also_image_of(&mut self, was: impl IntoIterator<Item = u64>) {
        self.was.extend(was);
    }
}

impl Held {
    /// Returns the id, `None` while it is an image without one.
    pub(super) fn id(&self) -> Option<u64> {
        IndexRef(self.key).id()
    }

    pub(super) fn kind(&self) -> IndexKind {
        self.kind
    }

    pub(super) fn elems(&self) -> &[Elem] {
        &self.elems
    }

    /// Returns the parts of the scope as names, `None` for every constructor.
    pub(super) fn scope_parts(&self) -> Option<Vec<Term>> {
        self.scope.terms().map(|(ts, _)| ts)
    }

    /// Returns each part of the scope as `model` spells it.
    pub(super) fn scope_names(&self, model: &Model) -> Vec<String> {
        self.scope_parts()
            .unwrap_or_default()
            .iter()
            .map(|p| p.print(model))
            .collect()
    }

    /// Returns the constructors the scope reaches, a named set's members in its place; `None`
    /// for every constructor.
    pub(super) fn reach(&self, model: &Model) -> Option<Vec<u64>> {
        if matches!(self.scope, ScopeIds::All) {
            return None;
        }
        let mut out = Vec::new();
        for o in self.scope.firsts().into_iter().flatten() {
            let more = match o {
                (Space::Set, set) => model.sets[&set]
                    .members
                    .iter()
                    .filter_map(|m| m.objects().next().map(|o| o.1))
                    .collect(),
                (_, c) => vec![c],
            };
            for c in more {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        Some(out)
    }

    /// Returns the constructors the scope lists, `None` when it names a set, a part naming no
    /// object, or every constructor.
    pub(super) fn listed_ctors(&self) -> Option<Vec<u64>> {
        if matches!(self.scope, ScopeIds::All) {
            return None;
        }
        self.scope
            .firsts()
            .into_iter()
            .map(|o| o.filter(|o| o.0 != Space::Set).map(|o| o.1))
            .collect()
    }

    /// Returns the constructors and named sets the scope names, as log clauses.
    pub(super) fn scope_clauses(&self) -> Vec<Clause> {
        self.scope
            .firsts()
            .into_iter()
            .flatten()
            .map(|o| match o {
                (Space::Set, s) => Clause::Scope { set: s },
                (_, c) => Clause::Member { ctor: c },
            })
            .collect()
    }

    /// Returns whether the scope names set `s`.
    pub(super) fn names_set(&self, s: u64) -> bool {
        match &self.scope {
            ScopeIds::All | ScopeIds::Ctors(..) => false,
            ScopeIds::Set(n) => n.id == s,
            ScopeIds::Written(ts, _) => {
                ts.iter().any(|p| p.objects().any(|o| o == (Space::Set, s)))
            }
        }
    }

    /// Returns whether an element names field `f`.
    pub(super) fn names_field(&self, f: u64) -> bool {
        self.elems
            .iter()
            .any(|e| e.objects().any(|o| o == (Space::Field, f)))
    }

    /// Returns the objects its elements and scope name.
    pub(super) fn objects(&self) -> Vec<Obj> {
        let mut out: Vec<Obj> = self.elems.iter().flat_map(Elem::objects).collect();
        if let Some(parts) = self.scope_parts() {
            out.extend(parts.iter().flat_map(Term::objects));
        }
        out
    }

    /// Returns the scope's parts with `by` in place of each constructor of `gone` it lists, each
    /// listed once, `None` when it lists none of `gone`.
    pub(super) fn rescoped(&self, model: &Model, gone: &[u64], by: &[u64]) -> Option<Vec<Term>> {
        let parts = self.scope_parts()?;
        let lists = parts.iter().any(|p| {
            p.objects()
                .next()
                .is_some_and(|o| o.0 == Space::Shared && gone.contains(&o.1))
        });
        if !lists {
            return None;
        }
        let mut out: Vec<Term> = Vec::new();
        let mut seen: Vec<Obj> = Vec::new();
        for p in &parts {
            let objs: Vec<Obj> = match p.objects().next() {
                Some(o) if o.0 == Space::Shared && gone.contains(&o.1) => {
                    by.iter().map(|&b| (Space::Shared, b)).collect()
                }
                Some(o) => vec![o],
                None => Vec::new(),
            };
            for o in objs {
                if seen.contains(&o) {
                    continue;
                }
                seen.push(o);
                out.push(if o.0 == Space::Shared && by.contains(&o.1) {
                    Term::name(o, &model.ctors[&o.1].name)
                } else {
                    p.clone()
                });
            }
        }
        Some(out)
    }

    /// Call `f` on each term of the elements and the scope.
    fn terms_mut(&mut self, f: &mut dyn FnMut(&mut Term)) {
        for e in &mut self.elems {
            f(&mut e.0);
        }
        if let Some((mut parts, listed)) = self.scope.terms() {
            parts.iter_mut().for_each(&mut *f);
            self.scope = ScopeIds::of(Some(parts), listed);
        }
    }
}

impl Indexes {
    fn get(&self, x: &IndexRef) -> &Held {
        self.0
            .iter()
            .find(|h| h.key == x.0)
            .expect("a clause marker names a unique or index its relation variable holds")
    }

    fn get_mut(&mut self, x: &IndexRef) -> &mut Held {
        self.0
            .iter_mut()
            .find(|h| h.key == x.0)
            .expect("a clause marker names a unique or index its relation variable holds")
    }

    /// Returns a key no unique or index of these holds, for an image.
    fn pending(&self) -> Key {
        let n = self
            .0
            .iter()
            .filter_map(|h| match h.key {
                Key::Pending(n) => Some(n + 1),
                Key::Id(_) => None,
            })
            .max()
            .unwrap_or(0);
        Key::Pending(n)
    }

    /// Hold `new`. Returns the marker of its clause.
    fn place(&mut self, new: NewIndex) -> ClauseM {
        let key = match new.id {
            Some(id) => Key::Id(id),
            None => self.pending(),
        };
        self.0.push(Held {
            key,
            kind: new.kind,
            scope: new.scope,
            elems: new.elems,
            was: new.was,
        });
        ClauseM::Index(IndexRef(key))
    }

    /// Call `f` on each term of each unique and index that prints as a term, with the position
    /// of its marker in `clauses`, in clause order: the elements, then a scope held as written.
    pub(super) fn lend_terms<'a>(
        &'a mut self,
        clauses: &[ClauseM],
        f: &mut dyn FnMut(usize, &'a mut Term),
    ) {
        let order: Vec<(usize, Key)> = clauses
            .iter()
            .enumerate()
            .filter_map(|(k, c)| match c {
                ClauseM::Index(x) => Some((k, x.0)),
                _ => None,
            })
            .collect();
        let held: BTreeMap<usize, &'a mut Held> = self
            .0
            .iter_mut()
            .filter_map(|h| {
                let at = order.iter().find(|(_, k)| *k == h.key)?.0;
                Some((at, h))
            })
            .collect();
        for (at, h) in held {
            for e in &mut h.elems {
                f(at, &mut e.0);
            }
            if let ScopeIds::Written(ts, _) = &mut h.scope {
                for t in ts {
                    f(at, t);
                }
            }
        }
    }

    /// Hold the unique or index `id` of kind `kind` a build declares over `elems` in scope
    /// `scope`, its parts and whether they are written in parentheses. Returns the marker of
    /// its clause.
    pub(super) fn lifted(
        &mut self,
        id: u64,
        kind: IndexKind,
        elems: Vec<Term>,
        scope: Option<(Vec<Term>, bool)>,
    ) -> ClauseM {
        let listed = scope.as_ref().is_some_and(|s| s.1);
        let scope = ScopeIds::of(scope.map(|s| s.0), listed);
        self.place(NewIndex {
            id: Some(id),
            kind,
            scope,
            elems: elems.into_iter().map(Elem).collect(),
            was: Vec::new(),
        })
    }
}

impl RelM {
    /// Returns each unique and index with the position of its clause, in clause order.
    pub(super) fn indexes(&self) -> impl Iterator<Item = (usize, &Held)> + '_ {
        self.clauses
            .iter()
            .enumerate()
            .filter_map(|(k, c)| match c {
                ClauseM::Index(x) => Some((k, self.indexes.get(x))),
                _ => None,
            })
    }

    /// Returns the unique or index of clause `at`, `None` when that clause is another.
    pub(super) fn index_at(&self, at: usize) -> Option<&Held> {
        match self.clauses.get(at)? {
            ClauseM::Index(x) => Some(self.indexes.get(x)),
            _ => None,
        }
    }

    /// Returns the position of the clause of unique or index `id`.
    pub(super) fn index_position(&self, id: u64) -> Option<usize> {
        self.clauses
            .iter()
            .position(|c| matches!(c, ClauseM::Index(x) if x.id() == Some(id)))
    }

    /// Append the clause of `new`.
    pub(super) fn push_index(&mut self, new: NewIndex) {
        let marker = self.indexes.place(new);
        self.clauses.push(marker);
    }

    /// Remove the clause at `at` and, when it is a unique or index, what it holds. Returns that
    /// unique or index.
    pub(super) fn remove_clause(&mut self, at: usize) -> Option<Held> {
        let ClauseM::Index(x) = self.clauses.remove(at) else {
            return None;
        };
        let k = self.indexes.0.iter().position(|h| h.key == x.0)?;
        Some(self.indexes.0.remove(k))
    }

    /// Remove the clauses at the positions `gone`.
    pub(super) fn remove_clauses(&mut self, gone: &BTreeSet<usize>) {
        for &at in gone.iter().rev() {
            self.remove_clause(at);
        }
    }

    /// Remove every unique and index.
    pub(super) fn drop_indexes(&mut self) {
        self.clauses.retain(|c| !matches!(c, ClauseM::Index(_)));
        self.indexes = Indexes::default();
    }

    /// Remove unique or index `id`. Returns false when this relation variable does not hold it.
    pub(super) fn remove_index_id(&mut self, id: u64) -> bool {
        match self.index_position(id) {
            Some(k) => {
                self.remove_clause(k);
                true
            }
            None => false,
        }
    }

    /// Returns whether this relation variable holds unique or index `id`.
    pub(super) fn holds_index(&self, id: u64) -> bool {
        self.index_position(id).is_some()
    }

    /// Leave the unique or index of clause `at` without an id, so that it takes a new one as the
    /// image of the clause it was.
    pub(super) fn reimage(&mut self, at: usize) {
        let RelM {
            clauses, indexes, ..
        } = self;
        let Some(ClauseM::Index(x)) = clauses.get_mut(at) else {
            return;
        };
        let Key::Id(old) = x.0 else {
            return;
        };
        let key = indexes.pending();
        let held = indexes.get_mut(x);
        held.key = key;
        if held.was.is_empty() {
            held.was.push(old);
        }
        x.0 = key;
    }

    /// Leave each unique and index `f` holds for without an id, as [`RelM::reimage`] does.
    pub(super) fn reimage_where(&mut self, f: impl Fn(&Held) -> bool) {
        let ats: Vec<usize> = self
            .indexes()
            .filter(|(_, h)| f(h))
            .map(|(k, _)| k)
            .collect();
        for at in ats {
            self.reimage(at);
        }
    }

    /// Hold the unique or index of clause `at` over `elems`.
    pub(super) fn carry_elems(&mut self, at: usize, elems: Vec<Term>) {
        if let Some(ClauseM::Index(x)) = self.clauses.get(at) {
            self.indexes.get_mut(x).elems = elems.into_iter().map(Elem).collect();
        }
    }

    /// Scope the unique or index of clause `at` to `parts`, giving it a new id as the image of
    /// the clause it was.
    pub(super) fn rescope(&mut self, at: usize, parts: Vec<Term>) {
        self.reimage(at);
        self.scope_to(at, parts);
    }

    /// Scope the unique or index of clause `at` to `parts`, keeping its id.
    pub(super) fn scope_to(&mut self, at: usize, parts: Vec<Term>) {
        if let Some(ClauseM::Index(x)) = self.clauses.get(at) {
            let listed = parts.len() > 1;
            self.indexes.get_mut(x).scope = ScopeIds::of(Some(parts), listed);
        }
    }

    /// Call `f` on each term of each unique and index.
    pub(super) fn index_terms_mut(&mut self, f: &mut dyn FnMut(&mut Term)) {
        for h in &mut self.indexes.0 {
            h.terms_mut(f);
        }
    }

    /// Give each key, generator, unique, index and computed field without an id the next of its
    /// space from `fresh`, in clause order, recording in `images` each image's new id with the
    /// ids of the clauses it is the image of.
    pub(super) fn identify(
        &mut self,
        fresh: &mut dyn FnMut(Space) -> u64,
        images: &mut BTreeMap<u64, Vec<u64>>,
    ) {
        let RelM {
            clauses, indexes, ..
        } = self;
        for c in clauses {
            match c {
                ClauseM::Key { id, generate, .. } => {
                    if id.is_none() {
                        *id = Some(fresh(Space::Index));
                    }
                    if let Some((_, g @ None)) = generate {
                        *g = Some(fresh(Space::Default));
                    }
                }
                ClauseM::Index(x) if x.id().is_none() => {
                    let new = fresh(Space::Index);
                    let held = indexes.get_mut(x);
                    held.key = Key::Id(new);
                    if !held.was.is_empty() {
                        images.insert(new, std::mem::take(&mut held.was));
                    }
                    x.0 = Key::Id(new);
                }
                ClauseM::Computed { id: id @ None, .. } => *id = Some(fresh(Space::Computed)),
                _ => {}
            }
        }
    }
}

impl Model {
    /// Print the clause of unique or index `x` of relation variable `rel`.
    pub(super) fn print_index(&self, rel: &RelM, x: &IndexRef, out: &mut Printed) {
        let h = rel.indexes.get(x);
        out.push(match h.kind {
            IndexKind::Unique => "unique (",
            IndexKind::Plain => "index (",
            IndexKind::Interval => "interval index (",
        });
        for (k, e) in h.elems.iter().enumerate() {
            if k > 0 {
                out.push(", ");
            }
            out.term(&e.0, self);
        }
        out.push(")");
        let (named, listed): (Vec<(Obj, &str)>, bool) = match &h.scope {
            ScopeIds::All => return,
            ScopeIds::Ctors(ns, listed) => (
                ns.iter()
                    .map(|n| ((Space::Shared, n.id), n.written.as_str()))
                    .collect(),
                *listed,
            ),
            ScopeIds::Set(n) => (vec![((Space::Set, n.id), n.written.as_str())], false),
            ScopeIds::Written(ts, listed) => {
                out.push(" in ");
                out.push(if *listed { "(" } else { "" });
                for (k, t) in ts.iter().enumerate() {
                    if k > 0 {
                        out.push(" | ");
                    }
                    out.term(t, self);
                }
                out.push(if *listed { ")" } else { "" });
                return;
            }
        };
        out.push(" in ");
        out.push(if listed { "(" } else { "" });
        for (k, (o, written)) in named.into_iter().enumerate() {
            if k > 0 {
                out.push(" | ");
            }
            out.push(self.name(o).unwrap_or(written));
        }
        out.push(if listed { ")" } else { "" });
    }
}
