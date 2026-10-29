use super::*;
use crate::kernel::Transition;

type Fields = BTreeMap<String, Value>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Change {
    pub rel: RelId,
    pub old: Option<Value>,
    pub new: Option<Value>,
}

pub(super) type Env = BTreeMap<Var, Value>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CopyToken(pub u64);

/// A relation in one database state, whose rows with equal keys are equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Keyed {
    pub rel: RelId,
    pub snapshot: u64,
    pub transition: Option<Transition>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Shape {
    #[default]
    Bottom,
    Scalar,
    Set(Box<Value>),
    Rows(BTreeMap<Option<u32>, Fields>),
    Change(Vec<Vec<Change>>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Value {
    pub reads: BTreeSet<Requirement>,
    pub copy: Option<CopyToken>,
    pub stored: BTreeSet<RelId>,
    pub keyed: Option<Keyed>,
    pub shape: Shape,
    pub may_empty: bool,
    pub binding: Option<Box<Value>>,
}

impl Value {
    pub fn scalar() -> Self {
        Self {
            shape: Shape::Scalar,
            ..Self::default()
        }
    }

    pub fn set(element: Self) -> Self {
        Self {
            shape: Shape::Set(Box::new(element)),
            may_empty: true,
            ..Self::default()
        }
    }

    pub fn singleton(element: Self) -> Self {
        Self {
            may_empty: false,
            ..Self::set(element)
        }
    }

    pub fn change(paths: Vec<Vec<Change>>) -> Self {
        Self {
            shape: Shape::Change(paths),
            ..Self::default()
        }
    }

    pub fn row(ctor: Option<u32>, fields: Fields) -> Self {
        Self {
            shape: Shape::Rows(BTreeMap::from([(ctor, fields)])),
            ..Self::default()
        }
    }

    pub fn elements(&self) -> Self {
        match &self.shape {
            Shape::Set(x) => (**x).clone(),
            _ => Self::default(),
        }
    }

    pub fn alternatives(&self) -> Vec<Self> {
        match &self.shape {
            Shape::Rows(rows) => rows
                .iter()
                .map(|(&ctor, fields)| Self {
                    reads: self.reads.clone(),
                    copy: self.copy,
                    stored: self.stored.clone(),
                    keyed: self.keyed,
                    may_empty: self.may_empty,
                    binding: None,
                    shape: Shape::Rows(BTreeMap::from([(ctor, fields.clone())])),
                })
                .collect(),
            Shape::Bottom => Vec::new(),
            _ => vec![self.clone()],
        }
    }

    pub fn field(&self, name: &str) -> Self {
        let mut out = Self::default();
        if let Shape::Rows(rows) = &self.shape {
            for fields in rows.values() {
                if let Some(value) = fields.get(name) {
                    out.join(value.clone());
                }
            }
        }
        out.reads.extend(self.reads.iter().cloned());
        out
    }

    pub fn restrict(&mut self, ctors: &[u32]) {
        match &mut self.shape {
            Shape::Rows(rows) => rows.retain(|c, _| c.is_none_or(|c| ctors.contains(&c))),
            Shape::Set(x) => x.restrict(ctors),
            Shape::Bottom | Shape::Scalar | Shape::Change(_) => {}
        }
    }

    pub fn each_read(&self, f: &mut impl FnMut(&Requirement)) {
        for r in &self.reads {
            f(r);
        }
        match &self.shape {
            Shape::Set(x) => x.each_read(f),
            Shape::Rows(rows) => {
                for value in rows.values().flat_map(BTreeMap::values) {
                    value.each_read(f);
                }
            }
            Shape::Bottom | Shape::Scalar | Shape::Change(_) => {}
        }
    }

    /// Call `f` with the relation and constructor of each stored row alternative that has no fields.
    pub fn each_fieldless(&self, f: &mut impl FnMut(RelId, u32)) {
        match &self.shape {
            Shape::Set(x) => x.each_fieldless(f),
            Shape::Rows(rows) => {
                for (ctor, fields) in rows {
                    if let (Some(c), true) = (ctor, fields.is_empty()) {
                        for &rel in &self.stored {
                            f(rel, *c);
                        }
                    }
                    for value in fields.values() {
                        value.each_fieldless(f);
                    }
                }
            }
            Shape::Bottom | Shape::Scalar | Shape::Change(_) => {}
        }
    }

    /// Drop the keyed marks of rows read in a snapshot numbered above `floor`.
    pub fn unkey_after(&mut self, floor: u64) {
        if self.keyed.is_some_and(|k| k.snapshot > floor) {
            self.keyed = None;
        }
        match &mut self.shape {
            Shape::Set(x) => x.unkey_after(floor),
            Shape::Rows(rows) => {
                for value in rows.values_mut().flat_map(BTreeMap::values_mut) {
                    value.unkey_after(floor);
                }
            }
            Shape::Bottom | Shape::Scalar | Shape::Change(_) => {}
        }
    }

    /// Call `f` with each read that can tell two such values apart, skipping keyed rows.
    pub fn each_distinguishing_read(&self, f: &mut impl FnMut(&Requirement)) {
        if self.keyed.is_some() {
            return;
        }
        for r in &self.reads {
            f(r);
        }
        match &self.shape {
            Shape::Set(x) => x.each_distinguishing_read(f),
            Shape::Rows(rows) => {
                for value in rows.values().flat_map(BTreeMap::values) {
                    value.each_distinguishing_read(f);
                }
            }
            Shape::Bottom | Shape::Scalar | Shape::Change(_) => {}
        }
    }

    pub fn join(&mut self, other: Self) {
        if matches!(self.shape, Shape::Bottom) {
            *self = other;
            return;
        }
        if matches!(other.shape, Shape::Bottom) {
            return;
        }
        self.reads.extend(other.reads);
        self.stored.extend(other.stored);
        self.may_empty |= other.may_empty;
        if let Some(binding) = other.binding {
            match &mut self.binding {
                Some(value) => value.join(*binding),
                None => self.binding = Some(binding),
            }
        }
        if self.copy != other.copy {
            self.copy = None;
        }
        if self.keyed != other.keyed {
            self.keyed = None;
        }
        match (&mut self.shape, other.shape) {
            (Shape::Set(a), Shape::Set(b)) => a.join(*b),
            (Shape::Rows(a), Shape::Rows(b)) => {
                for (ctor, fields) in b {
                    let dst = a.entry(ctor).or_default();
                    for (name, value) in fields {
                        dst.entry(name).or_default().join(value);
                    }
                }
            }
            (Shape::Scalar, Shape::Scalar) => {}
            (Shape::Change(a), Shape::Change(b)) => {
                for path in b {
                    if !a.contains(&path) {
                        a.push(path);
                    }
                }
            }
            _ => panic!("joined permission values have compatible checked types"),
        }
    }
}
