use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ast::{MigrationFile, Operator};
use adtb_lang::evolve::{self, Listed, OpSteps, RowMap, Step};
use adtb_lang::ids::{CatCtorId, CatIndexId};
use adtb_lang::schema::Schema;
use adtb_lang::translate::operator_text;

use super::ids::{ctor_id, type_ctor_id};
use super::{Broken, Error, ctor_count, ctor_relations};

/// Returns the schema `step` of operator `op` leaves, `text` the migration's text, or `None`
/// when that schema does not build and the step moves no rows; refuses the migration, naming the
/// operator, when it does not build and the step moves rows.
///
/// A step whose schema does not build and moves no rows folds into the next step that builds.
pub fn built(step: &Step, op: &Operator, text: &str) -> Result<Option<Schema>, Error> {
    match step.after.schema() {
        Ok(schema) => Ok(Some(schema)),
        Err(_) if step.maps.is_empty() => Ok(None),
        Err(ds) => {
            let shown = operator_text(text, op);
            Err(Error::Refused(
                ds.into_iter()
                    .map(|d| {
                        format!(
                            "`{shown}` moves rows into a schema that does not build: {}",
                            d.diag.message
                        )
                    })
                    .collect(),
            ))
        }
    }
}

/// Each operator of a migration in order with its steps, each step with the schema [`built`]
/// returns for it, a step it folds left out.
pub struct Stepped<'a> {
    ops: Vec<(Listed<'a>, Vec<(&'a Step, Schema)>)>,
}

impl<'a> Stepped<'a> {
    /// Returns each operator in order with its steps.
    pub fn ops(&self) -> impl Iterator<Item = (Listed<'a>, &[(&'a Step, Schema)])> + '_ {
        self.ops.iter().map(|(op, steps)| (*op, steps.as_slice()))
    }
}

impl Stepped<'_> {
    /// Returns the operator to blame for rows of constructors `rows`, one entry a row, that share
    /// a key of unique `id` at the migration's end, `before` the schema before it: the last
    /// operator after which, in every later state, the unique reaches every constructor two of
    /// those rows may have been of. Returns `None` when it reached them before the migration.
    pub fn widened(
        &self,
        before: &Schema,
        id: CatIndexId,
        rows: &[CatCtorId],
    ) -> Result<Option<usize>, Error> {
        let mut ids = BTreeSet::from([id]);
        let mut was: Vec<BTreeSet<CatCtorId>> = rows.iter().map(|c| BTreeSet::from([*c])).collect();
        let mut held = vec![false; self.ops.len()];
        for (k, (_, steps)) in self.ops.iter().enumerate().rev() {
            let after = self.after(k).unwrap_or(before);
            held[k] = collide(after, &ids, &was);
            for (i, (step, _)) in steps.iter().enumerate().rev() {
                let prior = match i {
                    0 => self.after_before(k).unwrap_or(before),
                    _ => &steps[i - 1].1,
                };
                ids = back_ids(prior, step, &ids);
                was = was.iter().map(|cs| back(prior, step, cs)).collect();
            }
        }
        if collide(before, &ids, &was) {
            return Ok(None);
        }
        match evolve::blamed(held.len(), |k| held[k]) {
            Some(k) => Ok(Some(k)),
            None => Err(Error::Internal(Box::new(Broken::UnreachedShared(id)))),
        }
    }

    /// Returns the schema after operator `k`, the last of a step before it when it has none.
    fn after(&self, k: usize) -> Option<&Schema> {
        self.ops[..=k]
            .iter()
            .rev()
            .find_map(|(_, steps)| steps.last().map(|(_, s)| s))
    }

    /// Returns the schema before operator `k`, when a step before it has one.
    fn after_before(&self, k: usize) -> Option<&Schema> {
        k.checked_sub(1).and_then(|j| self.after(j))
    }
}

/// Returns whether in `schema` the uniques of `ids` reach every constructor two rows of `rows`
/// may be of, each row by the constructors it may be of.
fn collide(schema: &Schema, ids: &BTreeSet<CatIndexId>, rows: &[BTreeSet<CatCtorId>]) -> bool {
    let reach = unique_reach(schema);
    let held: BTreeSet<&CatCtorId> = ids
        .iter()
        .filter_map(|id| reach.get(id))
        .flatten()
        .collect();
    rows.iter()
        .filter(|cs| cs.iter().all(|c| held.contains(c)))
        .count()
        > 1
}

/// Returns the uniques before `step`, which applies to `prior`, that those of `ids` after it may
/// be the image of.
fn back_ids(prior: &Schema, step: &Step, ids: &BTreeSet<CatIndexId>) -> BTreeSet<CatIndexId> {
    unique_reach(prior)
        .into_keys()
        .filter(|o| ids.contains(o) || ids.iter().any(|i| step.is_image_of(*i, *o)))
        .collect()
}

/// Returns the constructors before `step`, which applies to `prior`, whose rows a row of a
/// constructor of `ctors` after it may be the image of.
fn back(prior: &Schema, step: &Step, ctors: &BTreeSet<CatCtorId>) -> BTreeSet<CatCtorId> {
    let kept: BTreeSet<CatCtorId> = ctor_relations(prior)
        .into_iter()
        .map(|(r, c)| ctor_id(prior, r, c))
        .collect();
    let new = |t: &CatCtorId| !kept.contains(t);
    let mut out: BTreeSet<CatCtorId> = ctors.iter().filter(|t| kept.contains(t)).copied().collect();
    for map in &step.maps {
        let from: Vec<u32> = match map {
            RowMap::Route { from, .. } if ctors.iter().any(new) => from.clone(),
            RowMap::Merge { into, rows, .. } if ctors.contains(into) => {
                rows.iter().map(evolve::Merged::from).collect()
            }
            RowMap::Nest { into, from, .. } if ctors.contains(into) => {
                from.iter().map(|(c, _)| *c).collect()
            }
            RowMap::Unnest { from, products, .. }
                if products
                    .iter()
                    .any(|(a, b)| ctors.contains(a) || ctors.contains(b)) =>
            {
                vec![*from]
            }
            _ => Vec::new(),
        };
        if let Some(def) = map.def() {
            out.extend(from.into_iter().map(|c| type_ctor_id(prior, def, c)));
        }
    }
    out
}

/// Returns the constructors each key and unique of `schema` reaches, by its catalog id.
fn unique_reach(schema: &Schema) -> BTreeMap<CatIndexId, BTreeSet<CatCtorId>> {
    let all = |r| (0..ctor_count(schema, r)).map(move |c| ctor_id(schema, r, c));
    let keys = schema.ids().key_ids().map(|(r, id)| (id, all(r).collect()));
    let uniques = schema.ids().index_ids().filter_map(|((r, i), id)| {
        let def = &schema.relation(r).indexes[i];
        let ctors: BTreeSet<CatCtorId> = match &def.scope {
            Some(cs) => cs.iter().map(|&c| ctor_id(schema, r, c)).collect(),
            None => all(r).collect(),
        };
        def.unique.then_some((id, ctors))
    });
    keys.chain(uniques).collect()
}
/// Returns `steps`, the steps of each operator of `file` in order, `text` its text, each with
/// its schema.
///
/// Fail as [`built`] does, with [`Error::Refused`] at the first operator whose steps fail, and
/// with [`Broken::Unstepped`] unless `steps` holds one entry for each operator.
pub fn stepped<'a>(
    steps: &'a [OpSteps],
    file: &'a MigrationFile,
    text: &str,
) -> Result<Stepped<'a>, Error> {
    if steps.len() != file.operators.len() {
        return Err(Broken::Unstepped {
            operators: file.operators.len(),
            stepped: steps.len(),
        }
        .into());
    }
    let ops = Listed::all(&file.operators)
        .zip(steps)
        .map(|(op, steps)| {
            let steps = steps
                .as_ref()
                .map_err(|ds| Error::Refused(ds.iter().map(|d| d.message.clone()).collect()))?;
            let mut out = Vec::new();
            for step in steps {
                if let Some(schema) = built(step, &op, text)? {
                    out.push((step, schema));
                }
            }
            Ok((op, out))
        })
        .collect::<Result<_, Error>>()?;
    Ok(Stepped { ops })
}

#[cfg(test)]
mod tests {
    use adtb_lang::evolve::{RowMap, Source, SourceDecl, Step};
    use adtb_lang::parse::{Options, parse_migration};
    use adtb_lang::types::TypeId;

    use super::super::Error;
    use super::built;

    const TEXT: &str = "module app\nadd type Pair = { a: I64 }\n";

    fn unbuilt(maps: Vec<RowMap>) -> Step {
        let mut after = Source::new(vec![SourceDecl {
            module: "app".to_string(),
            text: "type Pair = { a: I64 }\n".to_string(),
        }])
        .expect("the source builds");
        after.decls[0].text = "type Pair = { a: Missing }\n".to_string();
        Step::new(after, maps, None, std::collections::BTreeMap::new())
    }

    #[test]
    fn a_step_moving_rows_into_a_schema_that_does_not_build_refuses_naming_its_operator() {
        let file = parse_migration(TEXT, Options::default())
            .ok()
            .expect("the migration parses");
        let pair = Source::new(vec![SourceDecl {
            module: "app".to_string(),
            text: "type Pair = { a: I64 }\n".to_string(),
        }])
        .expect("the source builds");
        let schema = pair.schema().expect("the source builds");
        let (_, a) = schema.ids().field_ids().next().expect("`a` has an id");
        let step = unbuilt(vec![RowMap::Copy {
            def: TypeId(0),
            copied: vec![(0, a, a)],
        }]);
        match built(&step, &file.operators[0], TEXT) {
            Err(Error::Refused(why)) => {
                assert!(
                    !why.is_empty()
                        && why
                            .iter()
                            .all(|w| w.starts_with("`add type Pair = { a: I64 }` moves rows")),
                    "{why:?}"
                );
            }
            other => panic!("refused, not {other:?}"),
        }
    }
}
