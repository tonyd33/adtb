use std::sync::{Arc, OnceLock};

use super::{Replaying, ResolvedOp, Source, model};
use crate::ast::Operator;
use crate::canon::Names;

/// A migration of the log, with the schema before it.
#[derive(Clone)]
pub struct Logged {
    number: u64,
    module: String,
    text: String,
    operators: Vec<Operator>,
    before: Arc<Source>,
    /// The migration applied again to `before`: its operators resolved and the names of its
    /// text, or why it does not apply. Filled from this migration's own fields only, and shared
    /// by every clone. Assumes the run height `norm::set_run_height` sets does not change while
    /// it lives.
    resolution: Arc<OnceLock<Replaying<Resolution>>>,
}

impl std::fmt::Debug for Logged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Logged")
            .field("number", &self.number)
            .field("module", &self.module)
            .field("text", &self.text)
            .field("operators", &self.operators)
            .field("before", &self.before)
            .finish()
    }
}

/// The operators of a migration of the log resolved, and the names of its text.
pub(super) type Resolution = (Vec<ResolvedOp>, Names);

impl Logged {
    pub(super) fn new(
        number: u64,
        module: String,
        text: String,
        operators: Vec<Operator>,
        before: Arc<Source>,
    ) -> Logged {
        Logged {
            number,
            module,
            text,
            operators,
            before,
            resolution: Arc::default(),
        }
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn module(&self) -> &str {
        &self.module
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn operators(&self) -> &[Operator] {
        &self.operators
    }

    /// Returns the source the migration applied to.
    pub fn before(&self) -> &Source {
        &self.before
    }

    /// Returns the migration applied again to the source before it, as
    /// [`Source::resolved`] and [`Logged::names`] give it, applying it once for this migration
    /// and every clone of it.
    pub(super) fn resolution(&self) -> Replaying<Resolution> {
        let resolve = || {
            let (_, names, got) = model::replay(self)?;
            Ok((got.resolved(), names?))
        };
        let Some(held) = self.resolution.get() else {
            return self.resolution.get_or_init(resolve).clone();
        };
        #[cfg(any(test, feature = "testing"))]
        if crate::sampled::REPLAYS.due() {
            assert_eq!(
                format!("{:?}", resolve()),
                format!("{held:?}"),
                "a held replay of migration {} equals the migration applied again",
                self.number
            );
        }
        held.clone()
    }
}
