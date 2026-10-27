use adtb_lang::ast::Operator;
use adtb_lang::evolve::Step;
use adtb_lang::schema::Schema;

use super::Error;

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
            let shown = text
                .get(op.span.start as usize..op.span.end as usize)
                .and_then(|s| s.lines().next())
                .unwrap_or_default()
                .trim();
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
        Step { after, maps }
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

    #[test]
    fn a_step_moving_no_rows_into_a_schema_that_does_not_build_folds_into_the_next() {
        let file = parse_migration(TEXT, Options::default())
            .ok()
            .expect("the migration parses");
        assert!(matches!(
            built(&unbuilt(Vec::new()), &file.operators[0], TEXT),
            Ok(None)
        ));
    }
}
