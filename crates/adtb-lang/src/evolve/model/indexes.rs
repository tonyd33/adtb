use std::borrow::Cow;
use std::ops::Deref;

use super::op::{Res, refuse};
use super::print::Printed;
use super::term::Bound;
use super::*;
use crate::ast::Expr;
use crate::canon::Names;
use crate::evolve::Clause;
use crate::ids::{KeyKind, ObjectKey, Owner};
use crate::schema::Adder;
use crate::schema::same::{self, Atom, Place, Said, Tree};

/// The clauses of a relation variable in written order, each unique and index held beside the
/// marker of its clause.
#[derive(Clone, Debug)]
pub(super) struct Clauses {
    list: Vec<ClauseM>,
    held: Vec<Held>,
}

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
    /// The constructors and named sets it names in written order, printed in parentheses when
    /// more than one.
    Parts(Vec<Part>),
}

/// A constructor or named set a scope names, with the name it prints once the model holds no
/// object of its id.
#[derive(Clone, Debug)]
pub(super) enum Part {
    Held {
        obj: Obj,
        written: String,
    },
    /// A named set an operator of the migration dropped, by its catalog key, until an operator
    /// adds a set of that key; `reach` is the constructors it held.
    Gone {
        key: ObjectKey,
        reach: Vec<u64>,
        written: String,
    },
}

/// A constructor or named set a scope names, by its id, or by its key once dropped.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PartId {
    Held(Obj),
    Gone(ObjectKey),
}

/// An element of a unique or index.
#[derive(Clone, Debug)]
pub(super) struct Elem {
    term: Term,
    /// The element's tree, each name the place of the piece of `term` holding it; `None` from an
    /// operator's rewrite of `term` until a build lifts it again.
    tree: Option<Tree<usize>>,
}

/// A part of the scope a drop writes: a constructor or named set, or the key of the named set of
/// its relation variable its name would be when no object answers to it.
#[derive(Clone, Debug)]
pub(super) enum DropPart {
    Held(Obj),
    Unheld(ObjectKey),
}

/// The trees of the elements a drop writes, `None` when one does not check.
type DropElems = Option<Vec<Tree<DropName>>>;

/// A name of a dropped set or function a unique or index holds at a migration's end.
pub(super) struct NamedGone {
    /// The catalog keys of the objects it named.
    keys: Vec<ObjectKey>,
    /// What kind of object it named, as a refusal words it.
    what: &'static str,
    written: String,
    /// The clause's text.
    clause: String,
    /// The relation variable and its name.
    relation: (u64, String),
}

impl NamedGone {
    pub(super) fn relation(&self) -> u64 {
        self.relation.0
    }

    pub(super) fn keys(&self) -> &[ObjectKey] {
        &self.keys
    }

    /// Returns the refusal of the clause for the name.
    pub(super) fn refusal(&self) -> String {
        format!(
            "`{}` of `{}` names the dropped {} `{}`",
            self.clause, self.relation.1, self.what, self.written
        )
    }
}

/// A name of an element a drop writes: the objects it resolves to, or, when it resolves to
/// none, its module, written or the drop's own, its name and where it stands.
#[derive(Clone, Debug)]
pub(super) enum DropName {
    Objects(BTreeSet<Obj>),
    Unheld {
        module: String,
        name: String,
        place: Place,
    },
}

/// What decides whether a unique or index is one with another: its kind, the constructors its
/// scope reaches, the constructors and sets it names, and its elements' trees.
struct Identity<N> {
    kind: IndexKind,
    reach: BTreeSet<u64>,
    written: Option<BTreeSet<PartId>>,
    trees: Vec<Tree<N>>,
}

impl<N> Identity<N> {
    /// Returns whether they are one, as `same::same_unique` and `same::same_index` decide,
    /// `name` deciding whether two names are one.
    fn same<M>(&self, other: &Identity<M>, name: &dyn Fn(&N, &M) -> bool) -> bool {
        let interval = |k: IndexKind| k == IndexKind::Interval;
        match (self.kind, other.kind) {
            (IndexKind::Unique, IndexKind::Unique) => same::same_unique(
                (&self.reach, &self.trees),
                (&other.reach, &other.trees),
                name,
            ),
            (IndexKind::Unique, _) | (_, IndexKind::Unique) => false,
            _ => same::same_index(
                (interval(self.kind), self.written.as_ref(), &self.trees),
                (interval(other.kind), other.written.as_ref(), &other.trees),
                name,
            ),
        }
    }
}

/// Returns whether `held`, a name of a held element, is `want`, one a drop writes: the same
/// objects, or a dropped object whose catalog key the unresolved name `want` spells, of a kind a
/// name where it stands names, owned by its module when a module owns it.
fn held_is(held: &Bound, want: &DropName) -> bool {
    match (held, want) {
        (Bound::Objects(a), DropName::Objects(b)) => a == b,
        (
            Bound::Keys(keys),
            DropName::Unheld {
                module,
                name,
                place,
            },
        ) => keys.iter().any(|k| {
            k.name() == Some(name.as_str())
                && names_kind(*place, k.kind())
                && match k.owner() {
                    Owner::Module(m) => m == module,
                    Owner::Object(..) => true,
                }
        }),
        _ => false,
    }
}

/// Returns whether a name standing at `place` can name an object of kind `kind`.
fn names_kind(place: Place, kind: KeyKind) -> bool {
    match place {
        Place::Value => matches!(
            kind,
            KeyKind::Function | KeyKind::Relation | KeyKind::Field | KeyKind::Computed
        ),
        Place::Callee => {
            names_kind(Place::Value, kind) || matches!(kind, KeyKind::Type | KeyKind::Ctor)
        }
        Place::Member => matches!(kind, KeyKind::Field | KeyKind::Computed),
        Place::Ctor => kind == KeyKind::Ctor,
        Place::Sel => matches!(kind, KeyKind::Ctor | KeyKind::Set),
        Place::Type => kind == KeyKind::Type,
        Place::Relation => kind == KeyKind::Relation,
    }
}

/// A unique or index an operator made, before its relation variable holds it.
#[derive(Clone)]
pub(super) struct NewIndex {
    id: Option<u64>,
    kind: IndexKind,
    scope: ScopeIds,
    elems: Vec<Elem>,
    was: Vec<u64>,
}

impl Deref for Clauses {
    type Target = [ClauseM];

    fn deref(&self) -> &[ClauseM] {
        &self.list
    }
}

impl<'a> IntoIterator for &'a Clauses {
    type Item = &'a ClauseM;
    type IntoIter = std::slice::Iter<'a, ClauseM>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

impl Part {
    /// Returns the part naming constructor or named set `obj`, written `written`.
    pub(super) fn held(obj: Obj, written: &str) -> Part {
        Part::Held {
            obj,
            written: written.to_string(),
        }
    }

    /// Returns the object it names, `None` for a dropped set.
    fn obj(&self) -> Option<Obj> {
        match self {
            Part::Held { obj, .. } => Some(*obj),
            Part::Gone { .. } => None,
        }
    }

    fn id(&self) -> PartId {
        match self {
            Part::Held { obj, .. } => PartId::Held(*obj),
            Part::Gone { key, .. } => PartId::Gone(key.clone()),
        }
    }

    /// Returns the constructors it reaches in `model`, a named set's members in its place;
    /// `None` for a set `model` does not hold.
    fn reach(&self, model: &Model) -> Option<Vec<u64>> {
        match self {
            Part::Held {
                obj: (Space::Set, s),
                ..
            } => model.sets.contains_key(s).then(|| model.set_reach(*s)),
            Part::Held { obj, .. } => Some(vec![obj.1]),
            Part::Gone { reach, .. } => Some(reach.clone()),
        }
    }

    /// Returns the part as `model` names it.
    fn print<'a>(&'a self, model: &'a Model) -> &'a str {
        match self {
            Part::Held { obj, written } => model.name(*obj).unwrap_or(written),
            Part::Gone { written, .. } => written,
        }
    }
}

impl ScopeIds {
    /// Returns the scope `parts` names, every constructor when `None`.
    fn of(parts: Option<Vec<Part>>) -> ScopeIds {
        match parts {
            None => ScopeIds::All,
            Some(ps) => ScopeIds::Parts(ps),
        }
    }

    fn parts(&self) -> &[Part] {
        match self {
            ScopeIds::All => &[],
            ScopeIds::Parts(ps) => ps,
        }
    }
}

impl Elem {
    /// Returns element `e` of `text`, each name in it held as `names` resolves it and each name
    /// `ahead` holds by what adds its relation variable, with its tree.
    pub(super) fn lift(
        text: &str,
        e: &Expr,
        names: &Names,
        ahead: &[(std::ops::Range<usize>, Adder)],
    ) -> Elem {
        let term = Term::lift(text, e.span.range(), names, ahead);
        let tree = term.tree(e.span.start as usize, e);
        Elem { term, tree }
    }

    fn written(term: Term) -> Elem {
        Elem { term, tree: None }
    }

    pub(super) fn term(&self) -> &Term {
        &self.term
    }

    /// Returns the objects the element names.
    pub(super) fn objects(&self) -> impl Iterator<Item = Obj> + '_ {
        self.term.objects()
    }

    /// Returns the element as `model` spells it.
    pub(super) fn print(&self, model: &Model) -> String {
        self.term.print(model)
    }

    /// Returns the element's tree, each name as what it resolves to now; `None` until a build
    /// lifts an element an operator rewrote.
    fn tree(&self) -> Option<Tree<Bound>> {
        self.tree
            .as_ref()?
            .iter()
            .map(|a| {
                Some(match a {
                    Atom::Name(k) => Atom::Name(self.term.bound_at(*k)?),
                    Atom::Bound(b) => Atom::Bound(*b),
                    Atom::Token(t) => Atom::Token(t.clone()),
                })
            })
            .collect()
    }

    /// Returns the first name of the element reading fields `model` names differently: its
    /// first field and its first field of another name.
    fn names_apart(&self, model: &Model) -> Option<(u64, u64)> {
        let name = |f: u64| model.fields.get(&f).map(|x| x.name.as_str());
        self.term.held_names().into_iter().find_map(|ids| {
            let mut fields = ids
                .iter()
                .filter(|o| o.0 == Space::Field && name(o.1).is_some())
                .map(|o| o.1);
            let first = fields.next()?;
            let other = fields.find(|&f| name(f) != name(first))?;
            Some((first, other))
        })
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
    /// Returns a unique or index of kind `kind` over `elems` in the scope `parts` names, every
    /// constructor when `None`, as the image of the clauses of ids `was`.
    pub(super) fn new(
        kind: IndexKind,
        elems: Vec<Term>,
        parts: Option<Vec<Part>>,
        was: Vec<u64>,
    ) -> NewIndex {
        NewIndex {
            id: None,
            kind,
            scope: ScopeIds::of(parts),
            elems: elems.into_iter().map(Elem::written).collect(),
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

    /// Returns the parts of the scope, `None` for every constructor.
    pub(super) fn parts(&self) -> Option<Vec<Part>> {
        match &self.scope {
            ScopeIds::All => None,
            ScopeIds::Parts(ps) => Some(ps.clone()),
        }
    }

    /// Returns each part of the scope as `model` spells it.
    pub(super) fn scope_names(&self, model: &Model) -> Vec<String> {
        self.scope
            .parts()
            .iter()
            .map(|p| p.print(model).to_string())
            .collect()
    }

    /// Returns the constructors the scope reaches, a named set's members in its place, a
    /// dropped set's members when it was dropped; `None` for every constructor. A set `model`
    /// does not hold reaches none.
    pub(super) fn reach(&self, model: &Model) -> Option<Vec<u64>> {
        if matches!(self.scope, ScopeIds::All) {
            return None;
        }
        let mut out = Vec::new();
        for p in self.scope.parts() {
            for c in p.reach(model).unwrap_or_default() {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        Some(out)
    }

    /// Returns the constructors the scope lists, `None` when it names a set or every
    /// constructor.
    pub(super) fn listed_ctors(&self) -> Option<Vec<u64>> {
        if matches!(self.scope, ScopeIds::All) {
            return None;
        }
        self.scope
            .parts()
            .iter()
            .map(|p| p.obj().filter(|o| o.0 == Space::Shared).map(|o| o.1))
            .collect()
    }

    /// Returns the constructors and named sets the scope names, as log clauses.
    pub(super) fn scope_clauses(&self) -> Vec<Clause> {
        self.scope
            .parts()
            .iter()
            .filter_map(Part::obj)
            .map(|o| match o {
                (Space::Set, s) => Clause::Scope { set: s },
                (_, c) => Clause::Member { ctor: c },
            })
            .collect()
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
        out.extend(self.scope.parts().iter().filter_map(Part::obj));
        out
    }

    /// Returns the scope's parts with `by` in place of each constructor of `gone` it lists, each
    /// listed once, `None` when it lists none of `gone`.
    pub(super) fn rescoped(&self, model: &Model, gone: &[u64], by: &[u64]) -> Option<Vec<Part>> {
        let parts = self.scope.parts();
        let lists = |p: &Part| {
            p.obj()
                .is_some_and(|o| o.0 == Space::Shared && gone.contains(&o.1))
        };
        if !parts.iter().any(lists) {
            return None;
        }
        let mut out: Vec<Part> = Vec::new();
        let mut seen: Vec<Obj> = Vec::new();
        for p in parts {
            let objs: Vec<Obj> = match p.obj() {
                Some(_) if lists(p) => by.iter().map(|&b| (Space::Shared, b)).collect(),
                Some(o) => vec![o],
                None => {
                    out.push(p.clone());
                    continue;
                }
            };
            for o in objs {
                if seen.contains(&o) {
                    continue;
                }
                seen.push(o);
                out.push(if o.0 == Space::Shared && by.contains(&o.1) {
                    Part::held(o, &model.ctors[&o.1].name)
                } else {
                    p.clone()
                });
            }
        }
        Some(out)
    }

    /// Returns whether a part of the scope is a set the migration dropped.
    fn names_gone_set(&self) -> bool {
        self.scope
            .parts()
            .iter()
            .any(|p| matches!(p, Part::Gone { .. }))
    }

    /// Returns whether it names an object the migration dropped: a set of its scope, or what a
    /// name of an element names.
    fn names_gone(&self) -> bool {
        self.names_gone_set() || self.elems.iter().any(|e| e.term.dropped())
    }

    /// Returns what makes it one with another, `every` the constructors of its relation variable
    /// in `model`; `None` when it names a set `model` does not hold, or has an element no index
    /// admits or one an operator rewrote that no build lifted again, which makes it one with
    /// none.
    fn identity(&self, model: &Model, every: &BTreeSet<u64>) -> Option<Identity<Bound>> {
        if self.scope.parts().iter().any(|p| p.reach(model).is_none()) {
            return None;
        }
        let trees = self
            .elems
            .iter()
            .map(Elem::tree)
            .collect::<Option<Vec<_>>>()?;
        Some(Identity {
            kind: self.kind,
            reach: match self.reach(model) {
                None => every.clone(),
                Some(cs) => cs.into_iter().collect(),
            },
            written: match &self.scope {
                ScopeIds::All => None,
                ScopeIds::Parts(ps) => Some(ps.iter().map(Part::id).collect()),
            },
            trees,
        })
    }

    /// Returns what makes the unique or index a drop writes one with this one, its kind `kind`,
    /// its scope `scope`, every constructor when `None`, and its elements' trees as `trees`
    /// gives them for the constructors that scope reaches, all when `None`; `None` when a part
    /// of `scope` names a set of this one's relation variable neither `model` nor this one's
    /// scope holds, as dropped, under that key.
    fn wanted(
        &self,
        model: &Model,
        every: &BTreeSet<u64>,
        (kind, scope): (IndexKind, Option<&[DropPart]>),
        trees: &mut dyn FnMut(Option<&BTreeSet<u64>>) -> DropElems,
    ) -> Option<Identity<DropName>> {
        let Some(scope) = scope else {
            return Some(Identity {
                kind,
                reach: every.clone(),
                written: None,
                trees: trees(None)?,
            });
        };
        let (mut reach, mut written) = (BTreeSet::new(), BTreeSet::new());
        for p in scope {
            let (id, cs) = match p {
                DropPart::Held(o) => {
                    let cs = match o.0 {
                        Space::Set => model.set_reach(o.1),
                        _ => vec![o.1],
                    };
                    (PartId::Held(*o), cs)
                }
                DropPart::Unheld(key) => {
                    let gone = self.scope.parts().iter().find_map(|q| match q {
                        Part::Gone { key: k, reach, .. } if k == key => Some(reach.clone()),
                        _ => None,
                    })?;
                    (PartId::Gone(key.clone()), gone)
                }
            };
            written.insert(id);
            reach.extend(cs);
        }
        let trees = trees(Some(&reach))?;
        Some(Identity {
            kind,
            reach,
            written: Some(written),
            trees,
        })
    }

    /// Hold each part of an object no longer `live` by the key `was` gives it, reaching the
    /// constructors `reach` gives it, and each dropped part whose key `now` holds by the object
    /// of that key.
    fn rebind(
        &mut self,
        (was, now): (&BTreeMap<Obj, ObjectKey>, &BTreeMap<ObjectKey, Obj>),
        live: &dyn Fn(Obj) -> bool,
        reach: &dyn Fn(Obj) -> Vec<u64>,
    ) {
        let ScopeIds::Parts(parts) = &mut self.scope else {
            return;
        };
        for p in parts {
            match p {
                Part::Held { obj, written } if !live(*obj) => {
                    if let Some(key) = was.get(obj) {
                        *p = Part::Gone {
                            key: key.clone(),
                            reach: reach(*obj),
                            written: key.name().unwrap_or(written).to_string(),
                        };
                    }
                }
                Part::Gone {
                    key,
                    written,
                    reach,
                    ..
                } => match now.get(key) {
                    Some(&obj) => *p = Part::held(obj, written),
                    None => reach.retain(|&c| live((Space::Shared, c))),
                },
                Part::Held { .. } => {}
            }
        }
    }
}

impl Clauses {
    pub(super) fn new() -> Clauses {
        Clauses {
            list: Vec::new(),
            held: Vec::new(),
        }
    }

    fn get(&self, x: &IndexRef) -> &Held {
        self.held
            .iter()
            .find(|h| h.key == x.0)
            .expect("a clause marker names a unique or index its relation variable holds")
    }

    /// Returns a key no unique or index of these holds, for an image.
    fn pending(&self) -> Key {
        let n = self
            .held
            .iter()
            .filter_map(|h| match h.key {
                Key::Pending(n) => Some(n + 1),
                Key::Id(_) => None,
            })
            .max()
            .unwrap_or(0);
        Key::Pending(n)
    }

    /// Hold `h` in place of the unique or index at position `at`.
    fn replace_held(&mut self, at: usize, h: Held) {
        let Some(ClauseM::Index(x)) = self.list.get_mut(at) else {
            return;
        };
        let old = std::mem::replace(&mut x.0, h.key);
        self.held.retain(|o| o.key != old && o.key != h.key);
        self.held.push(h);
    }

    /// Call `f` on the unique or index of clause `at`, when that clause is one.
    fn with_held(&mut self, at: usize, f: impl FnOnce(&mut Held)) {
        if let Some(ClauseM::Index(x)) = self.list.get(at) {
            let key = x.0;
            if let Some(h) = self.held.iter_mut().find(|h| h.key == key) {
                f(h);
            }
        }
    }

    /// Append the clause of `new`.
    fn place(&mut self, new: NewIndex) {
        let key = match new.id {
            Some(id) => Key::Id(id),
            None => self.pending(),
        };
        self.held.push(Held {
            key,
            kind: new.kind,
            scope: new.scope,
            elems: new.elems,
            was: new.was,
        });
        self.list.push(ClauseM::Index(IndexRef(key)));
    }

    /// Append the key over `fields`, written in parentheses when `listed`, with its id and its
    /// generator's.
    pub(super) fn push_key(
        &mut self,
        id: Option<u64>,
        fields: Vec<Term>,
        listed: bool,
        generate: Option<(String, Option<u64>)>,
    ) {
        self.list.push(ClauseM::Key {
            id,
            fields,
            listed,
            generate,
        });
    }

    pub(super) fn push_volatile(&mut self) {
        self.list.push(ClauseM::Volatile);
    }

    /// Append computed field `name` of type `ty` computing `expr`.
    pub(super) fn push_computed(&mut self, id: Option<u64>, name: String, ty: Term, expr: Term) {
        self.list.push(ClauseM::Computed { id, name, ty, expr });
    }

    /// Append the unique or index `id` of kind `kind` a build declares over `elems` in the scope
    /// `parts` names.
    pub(super) fn lifted(
        &mut self,
        id: u64,
        kind: IndexKind,
        elems: Vec<Elem>,
        parts: Option<Vec<Part>>,
    ) {
        self.place(NewIndex {
            id: Some(id),
            kind,
            scope: ScopeIds::of(parts),
            elems,
            was: Vec::new(),
        });
    }

    /// Call `f` on each term of each clause, in clause order: a key's fields, a unique's or
    /// index's elements, a computed field's type and expression. An element's tree waits for
    /// the next build to lift it, as `f` may rewrite its term.
    pub(super) fn lend_terms<'a>(&'a mut self, f: &mut dyn FnMut(&'a mut Term)) {
        for h in &mut self.held {
            for e in &mut h.elems {
                e.tree = None;
            }
        }
        self.each_term_mut(f);
    }

    /// Call `f` on each term of each clause, in clause order, as [`Clauses::lend_terms`] does.
    ///
    /// Preconditions:
    /// - `f` changes only what the names it holds resolve to, never the term's pieces.
    fn each_term_mut<'a>(&'a mut self, f: &mut dyn FnMut(&'a mut Term)) {
        let Clauses { list, held } = self;
        let mut held: Vec<Option<&'a mut Held>> = held.iter_mut().map(Some).collect();
        for c in list.iter_mut() {
            match c {
                ClauseM::Key { fields, .. } => fields.iter_mut().for_each(&mut *f),
                ClauseM::Index(x) => {
                    let found = held
                        .iter_mut()
                        .find(|h| h.as_ref().is_some_and(|h| h.key == x.0))
                        .and_then(Option::take);
                    for e in found.into_iter().flat_map(|h| &mut h.elems) {
                        f(&mut e.term);
                    }
                }
                ClauseM::Computed { ty, expr, .. } => {
                    f(ty);
                    f(expr);
                }
                ClauseM::Volatile => {}
            }
        }
    }
}

impl RelM {
    /// Returns each unique and index with the position of its clause, in clause order.
    pub(super) fn indexes(&self) -> impl Iterator<Item = (usize, &Held)> + '_ {
        self.clauses
            .list
            .iter()
            .enumerate()
            .filter_map(|(k, c)| match c {
                ClauseM::Index(x) => Some((k, self.clauses.get(x))),
                _ => None,
            })
    }

    /// Returns the unique or index of clause `at`, `None` when that clause is another.
    pub(super) fn index_at(&self, at: usize) -> Option<&Held> {
        match self.clauses.list.get(at)? {
            ClauseM::Index(x) => Some(self.clauses.get(x)),
            _ => None,
        }
    }

    /// Returns the position of the clause of unique or index `id`.
    pub(super) fn index_position(&self, id: u64) -> Option<usize> {
        self.clauses
            .list
            .iter()
            .position(|c| matches!(c, ClauseM::Index(x) if x.id() == Some(id)))
    }

    /// Append the clause of `new`.
    pub(super) fn push_index(&mut self, new: NewIndex) {
        self.clauses.place(new);
    }

    /// Append computed field `name` with id `id` of type `ty` computing `expr`.
    pub(super) fn push_computed(&mut self, id: u64, name: String, ty: Term, expr: Term) {
        self.clauses.push_computed(Some(id), name, ty, expr);
    }

    /// Hold computed clause `at` as of type `ty` computing `expr`. Returns false when clause
    /// `at` is no computed field.
    pub(super) fn redefine_computed(&mut self, at: usize, ty: Term, expr: Term) -> bool {
        let Some(ClauseM::Computed {
            ty: held_ty,
            expr: held_expr,
            ..
        }) = self.clauses.list.get_mut(at)
        else {
            return false;
        };
        *held_ty = ty;
        *held_expr = expr;
        true
    }

    /// Make the key one over `fields`, written in parentheses when `listed`, with id `id` and no
    /// generator. Returns false when the relation variable declares no key.
    pub(super) fn rekey(&mut self, id: u64, fields: Vec<Term>, listed: bool) -> bool {
        let Some(ClauseM::Key {
            id: held,
            fields: written,
            listed: l,
            generate,
        }) = self
            .clauses
            .list
            .iter_mut()
            .find(|c| matches!(c, ClauseM::Key { .. }))
        else {
            return false;
        };
        *held = Some(id);
        *written = fields;
        *l = listed;
        *generate = None;
        true
    }

    /// Put the key back over `fields`, written in parentheses when `listed`, generated by
    /// `generate` with the id its generator holds. Returns false when the relation variable
    /// declares no key.
    pub(super) fn restore_key(
        &mut self,
        fields: Vec<Term>,
        listed: bool,
        generate: Option<String>,
    ) -> bool {
        let Some(ClauseM::Key {
            fields: held,
            listed: l,
            generate: g,
            ..
        }) = self
            .clauses
            .list
            .iter_mut()
            .find(|c| matches!(c, ClauseM::Key { .. }))
        else {
            return false;
        };
        *held = fields;
        *l = listed;
        *g = generate.map(|x| {
            let id = g.as_ref().and_then(|(_, id)| *id);
            (x, id)
        });
        true
    }

    /// Remove the clause at `at` and, when it is a unique or index, what it holds. Returns that
    /// unique or index.
    pub(super) fn remove_clause(&mut self, at: usize) -> Option<Held> {
        let ClauseM::Index(x) = self.clauses.list.remove(at) else {
            return None;
        };
        let k = self.clauses.held.iter().position(|h| h.key == x.0)?;
        Some(self.clauses.held.remove(k))
    }

    /// Remove computed clause `at`. Returns its id, `None` when clause `at` is no computed
    /// field with one.
    pub(super) fn remove_computed(&mut self, at: usize) -> Option<u64> {
        let ClauseM::Computed { id: Some(id), .. } = self.clauses.list.get(at)? else {
            return None;
        };
        let id = *id;
        self.clauses.list.remove(at);
        Some(id)
    }

    /// Hold in place of each unique or index the one at its position in `was` that names a set
    /// the migration dropped, whose build printed that set as the constructors it reached.
    pub(super) fn keep_gone_scopes(&mut self, was: &RelM) {
        for (at, h) in was.indexes() {
            if h.names_gone_set() {
                self.clauses.replace_held(at, h.clone());
            }
        }
    }

    /// Make each dropped set a scope names reach `made` in place of the constructors `sources`.
    pub(super) fn reach_images(&mut self, sources: &[u64], made: &[u64]) {
        for h in &mut self.clauses.held {
            let ScopeIds::Parts(parts) = &mut h.scope else {
                continue;
            };
            for p in parts {
                let Part::Gone { reach, .. } = p else {
                    continue;
                };
                if !reach.iter().any(|c| sources.contains(c)) {
                    continue;
                }
                let mut now: Vec<u64> = Vec::new();
                for &c in reach.iter() {
                    let images = if sources.contains(&c) {
                        made
                    } else {
                        std::slice::from_ref(&c)
                    };
                    for &i in images {
                        if !now.contains(&i) {
                            now.push(i);
                        }
                    }
                }
                *reach = now;
            }
        }
    }

    /// Returns whether `want` spells the catalog key of an object the migration dropped that a
    /// unique or index of this relation variable names.
    pub(super) fn names_dropped(&self, want: &DropName) -> bool {
        self.clauses
            .held
            .iter()
            .flat_map(|h| &h.elems)
            .flat_map(|e| e.term.dropped_keys())
            .any(|b| held_is(&b, want))
    }

    /// Returns whether a unique or index names an object of one of `keys` the migration dropped.
    pub(super) fn names_gone_key(&self, keys: &[ObjectKey]) -> bool {
        self.clauses.held.iter().any(|h| {
            h.scope
                .parts()
                .iter()
                .any(|p| matches!(p, Part::Gone { key, .. } if keys.contains(key)))
                || h.elems.iter().any(|e| {
                    e.term
                        .dropped_names()
                        .any(|(ks, _)| ks.iter().any(|k| keys.contains(k)))
                })
        })
    }

    /// Remove the clauses at the positions `gone`.
    pub(super) fn remove_clauses(&mut self, gone: &BTreeSet<usize>) {
        for &at in gone.iter().rev() {
            self.remove_clause(at);
        }
    }

    /// Remove every unique and index.
    pub(super) fn drop_indexes(&mut self) {
        self.clauses
            .list
            .retain(|c| !matches!(c, ClauseM::Index(_)));
        self.clauses.held.clear();
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
        let Some(ClauseM::Index(IndexRef(Key::Id(old)))) = self.clauses.list.get(at) else {
            return;
        };
        let old = *old;
        let key = self.clauses.pending();
        self.clauses.with_held(at, |h| {
            h.key = key;
            if h.was.is_empty() {
                h.was.push(old);
            }
        });
        self.clauses.list[at] = ClauseM::Index(IndexRef(key));
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

    /// Hold the unique or index of clause `at` over `elems`. An element written as it was keeps
    /// its tree; the tree of any other waits for the next build to lift it.
    pub(super) fn carry_elems(&mut self, at: usize, elems: Vec<Term>) {
        self.clauses.with_held(at, |h| {
            let was = std::mem::take(&mut h.elems);
            h.elems = elems
                .into_iter()
                .enumerate()
                .map(|(k, term)| match was.get(k) {
                    Some(e) if e.term.written_alike(&term) => Elem {
                        term,
                        tree: e.tree.clone(),
                    },
                    _ => Elem::written(term),
                })
                .collect();
        });
    }

    /// Scope the unique or index of clause `at` to `parts`, giving it a new id as the image of
    /// the clause it was.
    pub(super) fn rescope(&mut self, at: usize, parts: Vec<Part>) {
        self.reimage(at);
        self.scope_to(at, parts);
    }

    /// Scope the unique or index of clause `at` to `parts`, keeping its id.
    pub(super) fn scope_to(&mut self, at: usize, parts: Vec<Part>) {
        self.clauses
            .with_held(at, |h| h.scope = ScopeIds::Parts(parts));
    }

    /// Call `f` on each term of each clause.
    ///
    /// Preconditions:
    /// - `f` changes only what the names it holds resolve to, never the term's pieces.
    pub(super) fn clause_terms_mut(&mut self, f: &mut dyn FnMut(&mut Term)) {
        self.clauses.each_term_mut(&mut |t| f(t));
    }

    /// Hold each scope part of an object no longer `live` by the key `was` gives it, reaching
    /// the constructors `reach` gives it, and each dropped part whose key `now` holds by the
    /// object of that key.
    pub(super) fn rebind_scopes(
        &mut self,
        keys: (&BTreeMap<Obj, ObjectKey>, &BTreeMap<ObjectKey, Obj>),
        live: &dyn Fn(Obj) -> bool,
        reach: &dyn Fn(Obj) -> Vec<u64>,
    ) {
        for h in &mut self.clauses.held {
            h.rebind(keys, live, reach);
        }
    }

    /// Returns whether a unique or index names a set the migration dropped.
    pub(super) fn names_gone(&self) -> bool {
        self.clauses.held.iter().any(Held::names_gone)
    }

    /// Give each key, generator, unique, index and computed field without an id the next of its
    /// space from `fresh`, in clause order, recording in `images` each image's new id with the
    /// ids of the clauses it is the image of.
    pub(super) fn identify(
        &mut self,
        fresh: &mut dyn FnMut(Space) -> u64,
        images: &mut BTreeMap<u64, Vec<u64>>,
    ) {
        let Clauses { list, held } = &mut self.clauses;
        for c in list {
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
                    if let Some(h) = held.iter_mut().find(|h| h.key == x.0) {
                        h.key = Key::Id(new);
                        if !h.was.is_empty() {
                            images.insert(new, std::mem::take(&mut h.was));
                        }
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
        let h = rel.clauses.get(x);
        out.push(match h.kind {
            IndexKind::Unique => "unique (",
            IndexKind::Plain => "index (",
            IndexKind::Interval => "interval index (",
        });
        for (k, e) in h.elems.iter().enumerate() {
            if k > 0 {
                out.push(", ");
            }
            out.term(&e.term, self);
        }
        out.push(")");
        let ScopeIds::Parts(parts) = &h.scope else {
            return;
        };
        let mut names: Vec<&str> = Vec::new();
        for p in parts {
            match p {
                Part::Gone { reach, .. } if !out.named() => {
                    for c in reach {
                        match self.ctors.get(c) {
                            Some(c) => names.push(&c.name),
                            None => out.broken(
                                "a dropped set's reach names a constructor the model lacks",
                            ),
                        }
                    }
                }
                _ => names.push(p.print(self)),
            }
        }
        let listed = names.len() > 1;
        out.push(" in ");
        out.push(if listed { "(" } else { "" });
        out.push(&names.join(" | "));
        out.push(if listed { ")" } else { "" });
    }

    /// Returns the constructors named set `set` holds.
    pub(super) fn set_reach(&self, set: u64) -> Vec<u64> {
        self.sets.get(&set).map_or(Vec::new(), |s| {
            s.members
                .iter()
                .filter_map(|m| m.objects().next().map(|o| o.1))
                .collect()
        })
    }

    /// Returns this model without the uniques and indexes naming an object the migration
    /// dropped.
    pub(super) fn without_gone(&self) -> Cow<'_, Model> {
        self.without(Held::names_gone)
    }

    fn without(&self, gone: fn(&Held) -> bool) -> Cow<'_, Model> {
        let any = |rel: &RelM| rel.indexes().any(|(_, h)| gone(h));
        if !self.relations.values().any(any) {
            return Cow::Borrowed(self);
        }
        let mut m = self.clone();
        for rel in m.relations.values_mut() {
            let gone: BTreeSet<usize> = rel
                .indexes()
                .filter(|(_, h)| gone(h))
                .map(|(k, _)| k)
                .collect();
            rel.remove_clauses(&gone);
        }
        Cow::Owned(m)
    }

    /// Returns the position of each clause of relation variable `r` holding the unique or index
    /// a drop writes, of kind `kind` in scope `scope`, every constructor when `None`, over the
    /// elements whose trees `trees` gives for the constructors a scope reaches, all when `None`.
    /// A name of the drop that resolves to no object is one only with an object of the clause
    /// it is compared with that the migration dropped, by that object's catalog key.
    pub(super) fn indexes_dropped(
        &self,
        r: u64,
        (kind, scope): (IndexKind, Option<&[DropPart]>),
        trees: &mut dyn FnMut(Option<&BTreeSet<u64>>) -> DropElems,
    ) -> Vec<usize> {
        let Some(rel) = self.relations.get(&r) else {
            return Vec::new();
        };
        let every: BTreeSet<u64> = self
            .relation_type(r)
            .map(|t| self.ctors_of(t).into_iter().collect())
            .unwrap_or_default();
        let mut seen: Vec<(Option<BTreeSet<u64>>, DropElems)> = Vec::new();
        let mut cached = |reach: Option<&BTreeSet<u64>>| {
            if let Some((_, t)) = seen.iter().find(|(r, _)| r.as_ref() == reach) {
                return t.clone();
            }
            let t = trees(reach);
            seen.push((reach.cloned(), t.clone()));
            t
        };
        rel.indexes()
            .filter(|(_, h)| {
                let Some(held) = h.identity(self, &every) else {
                    return false;
                };
                h.wanted(self, &every, (kind, scope), &mut cached)
                    .is_some_and(|w| held.same(&w, &held_is))
            })
            .map(|(k, _)| k)
            .collect()
    }

    /// Returns the text of the clause of unique or index `id` of relation variable `r` as this
    /// model prints it, `None` when it does not hold it.
    fn index_text(&self, r: u64, id: u64) -> Option<String> {
        let rel = self.relations.get(&r)?;
        let at = rel.index_position(id)?;
        Some(self.clause_text(rel, &rel.clauses[at]))
    }

    /// Refuse a unique or index one name of whose elements reads fields of two names, and two
    /// uniques or two indexes of one relation variable that are one, as `same::same_unique` and
    /// `same::same_index` decide. A refusal
    /// names the clause as `prior` printed it, or the clause it is the image of, when it held
    /// it; a collision names, as it is now, the one `prior` held as it is, else the earlier in
    /// clause order, and is refused as one declaration's when `prior` held neither nor a clause
    /// either is the image of. The uniques and indexes of a relation variable whose terms wait
    /// to be lifted again are left to the build that lifts them.
    pub(super) fn indexes_hold(&self, prior: &Model) -> Res<()> {
        let eq = |a: &Bound, b: &Bound| a == b;
        let shown = |r: u64, rel: &RelM, at: usize, h: &Held| -> String {
            let images = h.id().and_then(|id| self.images.get(&id));
            h.id()
                .into_iter()
                .chain(h.was.iter().copied())
                .chain(images.into_iter().flatten().copied())
                .find_map(|id| prior.index_text(r, id))
                .unwrap_or_else(|| self.clause_text(rel, &rel.clauses[at]))
        };
        for (&r, rel) in &self.relations {
            for (at, h) in rel.indexes() {
                let Some((f, g)) = h.elems.iter().find_map(|e| e.names_apart(self)) else {
                    continue;
                };
                let of = |x: u64| -> (&str, &str) {
                    let field = &self.fields[&x];
                    let owner = self.name((Space::Shared, field.owner)).unwrap_or_default();
                    (field.name.as_str(), owner)
                };
                let ((fname, fowner), (gname, gowner)) = (of(f), of(g));
                return refuse(format!(
                    "`{}` of `{}` names `{fname}` of `{fowner}` and `{gname}` of `{gowner}` as one element",
                    shown(r, rel, at, h),
                    rel.name
                ));
            }
        }
        for (&r, rel) in &self.relations {
            if self.unlifted.contains(&Item::Relation(r)) {
                continue;
            }
            let every = |m: &Model| -> BTreeSet<u64> {
                m.relations
                    .contains_key(&r)
                    .then(|| m.relation_type(r))
                    .flatten()
                    .map(|t| m.ctors_of(t).into_iter().collect())
                    .unwrap_or_default()
            };
            let (now, then) = (every(self), every(prior));
            let held_before = |h: &Held| {
                !h.was.is_empty()
                    || h.id().is_some_and(|id| {
                        self.images.contains_key(&id)
                            || prior.relations.get(&r).is_some_and(|x| x.holds_index(id))
                    })
            };
            let was = |h: &Held| -> Option<Identity<Bound>> {
                if prior.unlifted.contains(&Item::Relation(r)) {
                    return None;
                }
                let id = h.id()?;
                let held = prior.relations.get(&r)?;
                let (_, x) = held.indexes().find(|(_, x)| x.id() == Some(id))?;
                x.identity(prior, &then)
            };
            let all: Vec<_> = rel
                .indexes()
                .map(|(at, h)| (at, h, h.identity(self, &now), was(h)))
                .collect();
            for (i, (at, a, an, aw)) in all.iter().enumerate() {
                for (bt, b, bn, bw) in &all[..i] {
                    let (Some(an), Some(bn)) = (an, bn) else {
                        continue;
                    };
                    if !an.same(bn, &eq) {
                        continue;
                    }
                    if !held_before(b) && !held_before(a) {
                        return refuse(same::one_refusal(
                            Said::Declaration,
                            &rel.name,
                            &self.clause_text(rel, &rel.clauses[*bt]),
                            &self.clause_text(rel, &rel.clauses[*at]),
                        ));
                    }
                    let kept = |w: &Option<Identity<Bound>>, n: &Identity<Bound>| {
                        w.as_ref().is_some_and(|w| w.same(n, &eq))
                    };
                    let ((oat, other), hat) = if kept(aw, an) && !kept(bw, bn) {
                        ((*bt, *b), *at)
                    } else {
                        ((*at, *a), *bt)
                    };
                    return refuse(same::one_refusal(
                        Said::Migration,
                        &rel.name,
                        &shown(r, rel, oat, other),
                        &self.clause_text(rel, &rel.clauses[hat]),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Returns each name of a set or a function the migration dropped that a unique or index
    /// names, in relation variable and clause order, a clause's scope before its elements.
    pub(super) fn named_gone(&self) -> Vec<NamedGone> {
        let mut out = Vec::new();
        for (&r, rel) in &self.relations {
            for (at, h) in rel.indexes() {
                let mut named: Vec<(Vec<ObjectKey>, &'static str, String)> = Vec::new();
                for p in h.scope.parts() {
                    if let Part::Gone { key, written, .. } = p {
                        named.push((vec![key.clone()], "set", written.clone()));
                    }
                }
                for e in &h.elems {
                    for (keys, written) in e.term.dropped_names() {
                        let fns: Vec<ObjectKey> = keys
                            .iter()
                            .filter(|k| k.kind() == KeyKind::Function)
                            .cloned()
                            .collect();
                        if !fns.is_empty() {
                            named.push((fns, "function", written.to_string()));
                        }
                    }
                }
                for (keys, what, written) in named {
                    out.push(NamedGone {
                        keys,
                        what,
                        written,
                        clause: self.clause_text(rel, &rel.clauses[at]),
                        relation: (r, rel.name.clone()),
                    });
                }
            }
        }
        out
    }
}
