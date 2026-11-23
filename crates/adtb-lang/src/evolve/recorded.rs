use super::*;

/// A migration applied once to a source: the steps each operator takes and the operators
/// resolved.
#[derive(Debug)]
pub struct Recorded {
    steps: Vec<OpSteps>,
    resolved: Result<Vec<ResolvedOp>, Vec<Diag>>,
}

/// The steps of one operator, or why they fail.
pub type OpSteps = Result<Vec<Step>, Vec<Diag>>;

/// The source a migration leaves, its schema and checked program, and the migration as
/// [`Recorded`].
pub type Migrated = (Source, Compiled<Schema>, Recorded);

impl Recorded {
    /// Returns the steps of each operator in order, each taken from the source the operators
    /// before it leave, ending at the first operator whose steps fail.
    pub fn steps(&self) -> &[OpSteps] {
        &self.steps
    }

    /// Returns the operators as [`Source::resolved`] resolves them in the source the migration
    /// leaves.
    pub fn resolved(&self) -> Result<&[ResolvedOp], &[Diag]> {
        self.resolved.as_deref().map_err(Vec::as_slice)
    }
}

impl Source {
    /// Apply migration `m` as [`Source::accept`] does, or as [`Source::migrate`] does when not
    /// `accepting`, applying each operator once. Returns the source it leaves, checked, and the
    /// steps and resolved operators of that one application.
    pub fn record(
        &self,
        m: &MigrationFile,
        text: &str,
        accepting: bool,
    ) -> Result<Migrated, Failed> {
        let (next, checked, mut got) = self.migrating(m, text, &[], accepting, true)?;
        let steps = got.op_steps(m, text);
        let unavailable = next.unavailable(&m.operators);
        let resolved = match got.names() {
            _ if !unavailable.is_empty() => Err(unavailable),
            Some(Err(ds)) => Err(ds.clone()),
            _ => Ok(got.resolved()),
        };
        Ok((next, checked, Recorded { steps, resolved }))
    }

    /// Apply the operators of migration `m` as [`Source::migrate`] does, each once, without
    /// checking the result. Returns the steps of each operator as [`Recorded::steps`] does.
    ///
    /// Fail with the refusals of the operators that do not apply.
    pub fn record_steps(&self, m: &MigrationFile, text: &str) -> Result<Vec<OpSteps>, Failed> {
        let mut got = model::migrated(self, m, text, &[], false, false).map_err(Failed::Broken)?;
        let (blame, operators): (Vec<usize>, Vec<Diag>) = got.refusals(m).into_iter().unzip();
        if blame.is_empty() {
            return Ok(got.op_steps(m, text));
        }
        let mut result = got.source();
        result.settled();
        Err(Failed::Refused(Refusal {
            operators,
            blame,
            result: Some(Box::new(result)),
            ..Refusal::default()
        }))
    }

    /// Returns this source with `ops`, operators of module `module` whose text is `text`, applied
    /// in order, the result unchecked.
    ///
    /// Fail with the refusals of the operators that do not apply.
    pub fn declaring(&self, module: &str, ops: &[Operator], text: &str) -> Replaying<Source> {
        let m = MigrationFile {
            module: ast::Ident {
                text: module.to_string(),
                span: Span::default(),
            },
            operators: ops.to_vec(),
        };
        let got = model::migrated(self, &m, text, &[], false, false).map_err(Failed::Broken)?;
        let refused: Vec<Diag> = got.refusals(&m).into_iter().map(|(_, d)| d).collect();
        if !refused.is_empty() {
            return Err(Failed::Refused(refused));
        }
        let mut out = got.source();
        out.settled();
        Ok(out)
    }

    /// Returns why each of `ops` that adopts release data this source lacks cannot be resolved.
    pub(super) fn unavailable(&self, ops: &[Operator]) -> Vec<Diag> {
        ops.iter()
            .filter_map(|op| match &op.kind {
                OperatorKind::AdoptTzdb { from, .. } if !self.packages.contains_key(from) => {
                    Some(Diag::new(
                        op.span,
                        format!("the release data `{from}` is not available"),
                    ))
                }
                _ => None,
            })
            .collect()
    }
}
