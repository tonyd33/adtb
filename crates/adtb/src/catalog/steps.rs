use adtb_lang::ast::{MigrationFile, Operator};
use adtb_lang::evolve::{Listed, OpSteps, Step};
use adtb_lang::schema::Schema;
use adtb_lang::translate::operator_text;

use super::{Broken, Error};

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
        Step {
            after,
            maps,
            applied: None,
        }
    }

    #[test]
    fn a_step_moving_rows_into_a_schema_that_does_not_build_refuses_naming_its_operator() {
        let file = parse_migration(TEXT, Options::default())
            .ok()
            .expect("the migration parses");
        let step = unbuilt(vec![RowMap::Copy {
            def: TypeId(0),
            ctors: vec![0],
            field: "a".to_string(),
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
