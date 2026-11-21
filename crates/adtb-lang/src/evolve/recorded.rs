use super::names::Naming;
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
pub type Migrated = (Source, (Schema, Program), Recorded);

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
    ) -> Result<Migrated, Refusal> {
        let mut recorder = Recorder::new(true);
        let (next, checked) = self.migrating(m, text, &[], accepting, Some(&mut recorder))?;
        let Recorder {
            steps,
            naming,
            sources,
            ..
        } = recorder;
        let named = naming.map(|n| (n.into_names(), sources));
        let resolved = next.resolving(next.version, |_| named).map(|(ops, _)| ops);
        let recorded = Recorded { steps, resolved };
        #[cfg(feature = "compare-migration-model")]
        if model::enabled() {
            model::compare_recorded(self, m, text, &next, &recorded);
        }
        Ok((next, checked, recorded))
    }

    /// Apply the operators of migration `m` as [`Source::migrate`] does, each once, without
    /// checking the result. Returns the steps of each operator as [`Recorded::steps`] does.
    ///
    /// Fail with the refusals of the operators that do not apply.
    pub fn record_steps(&self, m: &MigrationFile, text: &str) -> Result<Vec<OpSteps>, Refusal> {
        let mut recorder = Recorder::new(false);
        let (next, mut refusal, _) = self.applying(m, text, &[], false, Some(&mut recorder));
        if refusal.operators.is_empty() {
            Ok(recorder.steps)
        } else {
            refusal.result = Some(Box::new(next));
            Err(refusal)
        }
    }
}

/// What applying the operators of a migration one at a time records while each applies.
pub(super) struct Recorder {
    steps: Vec<OpSteps>,
    naming: Result<Naming, Vec<Diag>>,
    /// Whether names and the sources before each operator are recorded.
    names: bool,
    /// The source before each operator, while recording names.
    sources: Vec<Source>,
    live: bool,
}

impl Recorder {
    fn new(names: bool) -> Recorder {
        Recorder {
            steps: Vec::new(),
            naming: Ok(Naming::default()),
            names,
            sources: Vec::new(),
            live: true,
        }
    }

    /// Prepare `src`, the source the first operator applies to, to record its copies when
    /// recording names.
    pub(super) fn start(&self, src: &mut Source) {
        if self.names {
            src.marks = Some(Marks::default());
        }
    }

    /// Apply operator `op` of a migration of `module` whose text is `text` to `src` as
    /// [`Source::apply`] does, recording its steps and names while every operator before it
    /// applied.
    pub(super) fn apply(
        &mut self,
        src: &mut Source,
        module: &str,
        op: Listed<'_>,
        text: &str,
    ) -> Result<(), Vec<Diag>> {
        if !self.live {
            return src.apply(module, op, text);
        }
        let prior = src.clone();
        let stepping = !matches!(self.steps.last(), Some(Err(_)));
        let noted = RefCell::new(Names::default());
        let noting = self.names.then_some(&noted);
        let recorded = RefCell::new(Vec::new());
        let steps =
            match src.apply_recording(module, op, text, noting, stepping.then_some(&recorded)) {
                Ok(()) if stepping => {
                    Some(prior.stepped(module, op, text, src, recorded.into_inner()))
                }
                Ok(()) => None,
                Err(ds) if !stepping => return Err(self.stop(src, ds)),
                Err(ds) => {
                    noted.take();
                    if let Err(again) = src.apply_recording(module, op, text, noting, None) {
                        return Err(self.stop(src, again));
                    }
                    Some(Err(ds))
                }
            };
        self.steps.extend(steps);
        if self.names {
            if let Ok(naming) = &mut self.naming {
                naming.noted(noted.into_inner());
                if let Err(ds) = naming.copied(&prior, src, op.span) {
                    self.naming = Err(ds);
                }
            }
            self.sources.push(prior);
        }
        Ok(())
    }

    /// Stop recording at an operator that does not apply, refused with `ds`.
    fn stop(&mut self, src: &mut Source, ds: Vec<Diag>) -> Vec<Diag> {
        self.live = false;
        src.marks = None;
        ds
    }

    /// End the recording at `src`, the source the last operator leaves.
    pub(super) fn end(&mut self, src: &mut Source) {
        if self.live
            && self.names
            && let Ok(naming) = &mut self.naming
            && let Err(ds) = naming.finish(src)
        {
            self.naming = Err(ds);
        }
        src.marks = None;
    }
}
