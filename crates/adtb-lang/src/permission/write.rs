use super::value::{Change, Changes, CopyToken, Shape};
use super::*;
use crate::kernel::Kind;

type Upsert = (RelId, Tm, Var, Var, Tm, bool);

impl Analyzer<'_> {
    pub(super) fn insert(
        &mut self,
        arena: Arena,
        rel: RelId,
        rows: Tm,
        env: &Env,
    ) -> Result<Value, Error> {
        let rows = self.eval(arena, rows, env)?;
        self.insert_rows(rel, &rows)
    }

    fn insert_rows(&mut self, rel: RelId, rows: &Value) -> Result<Value, Error> {
        self.consume(rows);
        let mut changes = Vec::new();
        for row in rows.elements().alternatives() {
            let ctor = constructor(&row)?;
            let target = self.target(rel, ctor)?;
            self.requirements
                .required
                .insert(Requirement::Insert(target));
            changes.push(Change {
                rel,
                old: None,
                new: Some(row),
            });
        }
        let mut out = Value::change(Changes::made(changes));
        out.binding = Some(Box::new(rows.clone()));
        Ok(out)
    }

    pub(super) fn delete(
        &mut self,
        arena: Arena,
        rel: RelId,
        rows: Tm,
        env: &Env,
    ) -> Result<Value, Error> {
        let rows = self.eval(arena, rows, env)?;
        let mut changes = Vec::new();
        for row in rows.elements().alternatives() {
            let target = self.target(rel, constructor(&row)?)?;
            self.requirements
                .required
                .insert(Requirement::Delete(target));
            changes.push(Change {
                rel,
                old: Some(row),
                new: None,
            });
        }
        Ok(Value::change(Changes::made(changes)))
    }

    pub(super) fn replace(
        &mut self,
        arena: Arena,
        rel: RelId,
        rows: Tm,
        function: Tm,
        env: &Env,
    ) -> Result<Value, Error> {
        let rows = self.eval(arena, rows, env)?;
        let Kind::Lam(var, body) = *self.terms(arena).kind(function) else {
            return Err(Error::InvalidKernel("replace retains its row lambda"));
        };
        let mut changes = Vec::new();
        for mut old in rows.elements().alternatives() {
            self.old_copies(&mut old);
            let new = self.bind_eval(arena, var, old.clone(), body, env)?;
            for new in new.alternatives() {
                self.replacement(rel, &old, &new)?;
                changes.push(Change {
                    rel,
                    old: Some(old.clone()),
                    new: Some(new),
                });
            }
        }
        let mut bound = Value::default();
        for change in &changes {
            if let Some(new) = &change.new {
                bound.join(new.clone());
            }
        }
        let mut effects = Changes::made(changes);
        if !rows.may_empty {
            effects.replaced = effects
                .made
                .iter()
                .filter_map(|change| Some((rel, change.old.clone()?)))
                .collect();
        }
        let mut out = Value::change(effects);
        out.binding = Some(Box::new(Value::set(bound)));
        Ok(out)
    }

    fn old_copies(&mut self, row: &mut Value) {
        if let Shape::Rows(rows) = &mut row.shape {
            for value in rows.values_mut().flat_map(BTreeMap::values_mut) {
                value.copy = Some(CopyToken(self.fresh()));
            }
        }
        if row.copy.is_none() {
            row.copy = Some(CopyToken(self.fresh()));
        }
    }

    fn replacement(&mut self, rel: RelId, old: &Value, new: &Value) -> Result<(), Error> {
        let from = constructor(old)?;
        let to = constructor(new)?;
        let target = self.target(rel, from)?;
        if from != to {
            let destination = self.target(rel, to)?;
            self.requirements.required.insert(Requirement::Retag {
                relation: target.relation,
                from: target.constructor,
                to: destination.constructor,
            });
        }
        let Shape::Rows(rows) = &new.shape else {
            return Err(Error::InvalidKernel("replacement is a row"));
        };
        for (name, value) in rows.values().flat_map(BTreeMap::iter) {
            let previous = old.field(name);
            let same = value.copy.is_some() && value.copy == previous.copy;
            if !same {
                self.consume(value);
                if from == to && !self.schema.relation(rel).key.contains(name) {
                    let field = self.field_id(rel, from, name)?;
                    self.requirements
                        .required
                        .insert(Requirement::UpdateField(target, field));
                }
            }
        }
        Ok(())
    }

    pub(super) fn upsert(
        &mut self,
        arena: Arena,
        upsert: Upsert,
        env: &Env,
    ) -> Result<Value, Error> {
        let (rel, rows, old_var, new_var, otherwise, single) = upsert;
        let proposed = self.eval(arena, rows, env)?;
        self.insert_rows(rel, &proposed)?;
        let Ty::Data { ctors, .. } = self.terms(arena).var_info(old_var).ty.clone() else {
            return Err(Error::InvalidKernel("upsert old has a row type"));
        };
        let old = self
            .relation(rel, ctors.as_deref(), None, false)?
            .elements();
        let mut bound = proposed.elements();
        let mut effects = Changes::made(Vec::new());
        for new in proposed.elements().alternatives() {
            effects.made.push(Change {
                rel,
                old: None,
                new: Some(new.clone()),
            });
            for old in old.alternatives() {
                let mut inner = env.clone();
                inner.insert(old_var, old.clone());
                inner.insert(new_var, new.clone());
                let result = self.eval(arena, otherwise, &inner)?;
                if let Shape::Change(changes) = result.shape {
                    bound.join(upsert_bound(rel, &old, &changes));
                    effects.or(changes);
                }
            }
        }
        let mut out = Value::change(effects);
        out.binding = Some(Box::new(if single { bound } else { Value::set(bound) }));
        Ok(out)
    }

    pub(super) fn parallel(
        &mut self,
        arena: Arena,
        writes: &[Tm],
        env: &Env,
    ) -> Result<Value, Error> {
        let mut out = Changes::made(Vec::new());
        for &write in writes {
            let value = self.eval(arena, write, env)?;
            out.then(match value.shape {
                Shape::Change(next) => next,
                Shape::Bottom => Changes::default(),
                _ => return Err(Error::InvalidKernel("parallel joins writes")),
            });
        }
        Ok(Value::change(out))
    }
}

fn constructor(value: &Value) -> Result<u32, Error> {
    match &value.shape {
        Shape::Rows(rows) if rows.len() == 1 => rows
            .first_key_value()
            .and_then(|(c, _)| *c)
            .ok_or(Error::InvalidKernel("relation row has a constructor")),
        _ => Err(Error::InvalidKernel(
            "write analyzes one constructor alternative",
        )),
    }
}

/// Returns what an upsert into `rel` binds for the conflicting row `old` when its otherwise
/// branch has `changes`: the rows that replace `old` or are inserted, and `old` unless every
/// completion replaces it.
fn upsert_bound(rel: RelId, old: &Value, changes: &Changes) -> Value {
    let mut out = Value::default();
    if !changes.reachable {
        return out;
    }
    for change in changes.made.iter().filter(|c| c.rel == rel) {
        if let Some(new) = &change.new
            && change.old.as_ref().is_none_or(|r| same_row(r, old))
        {
            out.join(new.clone());
        }
    }
    let replaced = changes
        .replaced
        .iter()
        .any(|(r, row)| *r == rel && same_row(row, old));
    if !replaced {
        out.join(old.clone());
    }
    out
}

fn same_row(a: &Value, b: &Value) -> bool {
    if a.copy.is_some() && a.copy == b.copy {
        return true;
    }
    match (&a.shape, &b.shape) {
        (Shape::Rows(a), Shape::Rows(b)) if a.len() == 1 && a.keys().eq(b.keys()) => {
            a.values().flat_map(BTreeMap::iter).all(|(name, value)| {
                let other = b.values().next().and_then(|fields| fields.get(name));
                value.copy.is_some() && other.is_some_and(|other| other.copy == value.copy)
            })
        }
        _ => false,
    }
}
