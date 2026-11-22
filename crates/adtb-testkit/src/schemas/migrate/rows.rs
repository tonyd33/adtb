//! The rows of a case by constructor and field name, and what each drawn operator does to them.

use std::collections::{BTreeMap, BTreeSet};

use adtb_reference::Value;

use super::super::{Case, Rel, Ty};

/// A row: its constructor's name and each field's value by name, the header's among them.
type Row = (String, BTreeMap<String, Value>);

/// The rows of each relation of a case, by relation name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows(BTreeMap<String, BTreeSet<Row>>);

impl Rows {
    /// Returns the rows of relation `rel`.
    pub fn of(&self, rel: &str) -> Option<&BTreeSet<Row>> {
        self.0.get(rel)
    }

    fn rel(&mut self, rel: &str) -> Result<&mut BTreeSet<Row>, String> {
        self.0
            .get_mut(rel)
            .ok_or_else(|| format!("the model holds no relation `{rel}`"))
    }

    /// Replace each row of relation `rel` by what `f` makes of it.
    fn map(&mut self, rel: &str, mut f: impl FnMut(Row) -> Row) -> Result<(), String> {
        let rows = self.rel(rel)?;
        *rows = std::mem::take(rows).into_iter().map(&mut f).collect();
        Ok(())
    }
}

/// What an operator does to the rows it finds.
#[derive(Clone, Debug)]
pub(super) enum Effect {
    None,
    AddRel(String),
    DropRel(String),
    RenameRel(String, String),
    RenameCtor {
        rel: String,
        from: String,
        to: String,
    },
    RenameField {
        rel: String,
        ctor: String,
        from: String,
        to: String,
    },
    /// Rows of `ctor` go to the product the routing names: with a flag, the first product when
    /// it holds `true`, the second otherwise, the flag dropped; without one, the one product.
    Split {
        rel: String,
        ctor: String,
        flag: Option<String>,
        products: Vec<String>,
    },
    /// Rows of each source go to `product`, `field` holding the nullary constructor at the
    /// source's position among `sources`.
    Merge {
        rel: String,
        sources: Vec<String>,
        product: String,
        field: String,
    },
    /// Rows of `ctor` go to the constructor `field` holds, at its position among `ctors`,
    /// without `field`.
    Unnest {
        rel: String,
        ctor: String,
        field: String,
        ctors: Vec<String>,
    },
    /// Rows of `ctor`, or of every constructor when `None`, gain `field` holding `value`.
    AddField {
        rel: String,
        ctor: Option<String>,
        field: String,
        value: Literal,
    },
    /// Rows of `ctor`, or of every constructor when `None`, lose `field`.
    DropField {
        rel: String,
        ctor: Option<String>,
        field: String,
    },
    /// Rows of every constructor but `except` gain `field` holding `value`.
    Fill {
        rel: String,
        except: String,
        field: String,
        value: Literal,
    },
    /// Rows of every constructor outside `to` lose `field`.
    MoveDown {
        rel: String,
        field: String,
        to: Vec<String>,
    },
    /// Rows of `ctor` are dropped.
    RemoveCtor {
        rel: String,
        ctor: String,
    },
    /// The model does not follow the operator's rows.
    Unknown(String),
}

impl Effect {
    /// Take relation `rel` and constructors `cs` of it to the relation and constructors this
    /// effect sends their rows to. Fails on an effect the model does not follow.
    pub(super) fn ctors(&self, rel: &mut String, cs: &mut BTreeSet<String>) -> Result<(), String> {
        let mut replace = |r: &str, from: &[String], to: &[String]| {
            if *rel == r && from.iter().filter(|c| cs.remove(*c)).count() > 0 {
                cs.extend(to.iter().cloned());
            }
        };
        match self {
            Effect::RenameRel(from, to) if rel == from => to.clone_into(rel),
            Effect::DropRel(r) if rel == r => cs.clear(),
            Effect::RenameCtor { rel: r, from, to } => {
                replace(r, std::slice::from_ref(from), std::slice::from_ref(to));
            }
            Effect::Split {
                rel: r,
                ctor,
                products,
                ..
            } => replace(r, std::slice::from_ref(ctor), products),
            Effect::Merge {
                rel: r,
                sources,
                product,
                ..
            } => replace(r, sources, std::slice::from_ref(product)),
            Effect::Unnest {
                rel: r,
                ctor,
                ctors,
                ..
            } => replace(r, std::slice::from_ref(ctor), ctors),
            Effect::RemoveCtor { rel: r, ctor } => replace(r, std::slice::from_ref(ctor), &[]),
            Effect::Unknown(why) => return Err(why.clone()),
            _ => {}
        }
        Ok(())
    }

    /// Take `field`, of constructors `cs` of relation `rel`, to the name this effect gives it.
    pub(super) fn rename_field(&self, rel: &str, cs: &BTreeSet<String>, field: &mut String) {
        if let Effect::RenameField {
            rel: r,
            ctor,
            from,
            to,
        } = self
            && r == rel
            && cs.contains(ctor)
            && from == field
        {
            to.clone_into(field);
        }
    }

    /// Apply the effect to `rows`. Fails on an effect the model does not follow.
    pub(super) fn apply(&self, rows: &mut Rows) -> Result<(), String> {
        match self {
            Effect::None => {}
            Effect::AddRel(r) => {
                rows.0.insert(r.clone(), BTreeSet::new());
            }
            Effect::DropRel(r) => {
                rows.0.remove(r);
            }
            Effect::RenameRel(from, to) => {
                let held = rows.0.remove(from).unwrap_or_default();
                rows.0.insert(to.clone(), held);
            }
            Effect::RenameCtor { rel, from, to } => {
                rows.map(rel, |(c, fs)| (if c == *from { to.clone() } else { c }, fs))?;
            }
            Effect::RemoveCtor { rel, ctor } => {
                rows.rel(rel)?.retain(|(c, _)| c != ctor);
            }
            Effect::RenameField {
                rel,
                ctor,
                from,
                to,
            } => rows.map(rel, |(c, mut fs)| {
                if c == *ctor
                    && let Some(v) = fs.remove(from)
                {
                    fs.insert(to.clone(), v);
                }
                (c, fs)
            })?,
            Effect::Split {
                rel,
                ctor,
                flag,
                products,
            } => rows.map(rel, |(c, mut fs)| {
                if c != *ctor {
                    return (c, fs);
                }
                let to = match flag {
                    Some(b) => {
                        let set = fs.remove(b) == Some(Value::bool(true));
                        &products[usize::from(!set)]
                    }
                    None => &products[0],
                };
                (to.clone(), fs)
            })?,
            Effect::Merge {
                rel,
                sources,
                product,
                field,
            } => rows.map(rel, |(c, mut fs)| {
                match sources.iter().position(|s| *s == c) {
                    Some(k) => {
                        fs.insert(field.clone(), Value::Ctor(k as u32, Vec::new()));
                        (product.clone(), fs)
                    }
                    None => (c, fs),
                }
            })?,
            Effect::Unnest {
                rel,
                ctor,
                field,
                ctors,
            } => rows.map(rel, |(c, mut fs)| {
                if c != *ctor {
                    return (c, fs);
                }
                match fs.remove(field) {
                    Some(Value::Ctor(k, _)) => (ctors[k as usize].clone(), fs),
                    other => (format!("{c} holding {other:?} in `{field}`"), fs),
                }
            })?,
            Effect::AddField {
                rel,
                ctor,
                field,
                value,
            } => rows.map(rel, |(c, mut fs)| {
                if ctor.as_ref().is_none_or(|d| *d == c) {
                    fs.insert(field.clone(), value.value());
                }
                (c, fs)
            })?,
            Effect::DropField { rel, ctor, field } => rows.map(rel, |(c, mut fs)| {
                if ctor.as_ref().is_none_or(|d| *d == c) {
                    fs.remove(field);
                }
                (c, fs)
            })?,
            Effect::Fill {
                rel,
                except,
                field,
                value,
            } => rows.map(rel, |(c, mut fs)| {
                if c != *except {
                    fs.insert(field.clone(), value.value());
                }
                (c, fs)
            })?,
            Effect::MoveDown { rel, field, to } => rows.map(rel, |(c, mut fs)| {
                if !to.contains(&c) {
                    fs.remove(field);
                }
                (c, fs)
            })?,
            Effect::Unknown(what) => return Err(what.clone()),
        }
        Ok(())
    }
}

/// A literal a fill gives.
#[derive(Clone, Debug)]
pub(super) enum Literal {
    Int(i64),
    Text(&'static str),
    Bool(bool),
    /// The first constructor of a sum of nullary constructors.
    First,
    None,
}

impl Literal {
    fn value(&self) -> Value {
        match self {
            Literal::Int(n) => Value::Int(*n),
            Literal::Text(t) => Value::Text((*t).into()),
            Literal::Bool(b) => Value::bool(*b),
            Literal::First => Value::Ctor(0, Vec::new()),
            Literal::None => Value::none(),
        }
    }
}

/// Returns the literal `other` of [`super::super::literal`] gives for `ty`.
pub(super) fn literal_value(ty: &Ty, other: bool) -> Literal {
    match (ty, other) {
        (Ty::Int, false) => Literal::Int(5),
        (Ty::Int, true) => Literal::Int(-3),
        (Ty::Text, false) => Literal::Text("p"),
        (Ty::Text, true) => Literal::Text("a"),
        (Ty::Bool, o) => Literal::Bool(o),
        (Ty::Sum(_) | Ty::Tag(..), _) => Literal::First,
        (Ty::OptInt | Ty::OptRef(..), _) => Literal::None,
        (Ty::Ref(..), _) => unreachable!("a literal gives no reference"),
    }
}

/// Returns the names of the fields of constructor `c` of `r`, header first, each in
/// declaration order, as a row value holds them.
fn field_names(r: &Rel, c: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if r.keys == 0 {
        out.push("id".into());
    }
    out.extend(r.header.iter().map(|f| f.name.clone()));
    out.extend(r.ctors[c].fields.iter().map(|f| f.name.clone()));
    out
}

impl Case {
    /// Returns the rows of `contents`, the rows of each relation of the case in its order.
    pub fn rows_of(&self, contents: &[Vec<Value>]) -> Result<Rows, String> {
        let mut out = Rows::default();
        for (r, rows) in self.schema.rels.iter().zip(contents) {
            let mut held = BTreeSet::new();
            for row in rows {
                let Value::Ctor(c, fs) = row else {
                    return Err(format!("a row of `{}` is no constructor: {row:?}", r.name));
                };
                let Some(ctor) = r.ctors.get(*c as usize) else {
                    return Err(format!(
                        "a row of `{}` has constructor {c}: {row:?}",
                        r.name
                    ));
                };
                held.insert((ctor.name.clone(), fs.iter().cloned().collect()));
            }
            out.0.insert(r.name.clone(), held);
        }
        Ok(out)
    }

    /// Returns the rows of each relation of the case as values, each by its constructor's index,
    /// or an error on a row of a constructor or with fields the case does not declare.
    pub fn values(&self, rows: &Rows) -> Result<Vec<Vec<(u32, Value)>>, String> {
        let mut out = Vec::new();
        for r in &self.schema.rels {
            let mut values = Vec::new();
            for (ctor, fs) in rows.of(&r.name).into_iter().flatten() {
                let c = r
                    .ctors
                    .iter()
                    .position(|d| d.name == *ctor)
                    .ok_or_else(|| format!("`{}` has no constructor `{ctor}`", r.name))?;
                let names = field_names(r, c);
                let held: BTreeSet<&String> = fs.keys().collect();
                if held != names.iter().collect() {
                    return Err(format!(
                        "a row of `{}.{ctor}` holds fields {held:?}, not {names:?}",
                        r.name
                    ));
                }
                let fields = names.iter().map(|n| (n.clone(), fs[n].clone())).collect();
                values.push((c as u32, Value::Ctor(c as u32, fields)));
            }
            out.push(values);
        }
        Ok(out)
    }
}
