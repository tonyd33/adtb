use std::sync::Arc;

use super::{Adhoc, Program};
use crate::schema::{Committed, Schema};

/// A schema, owned or borrowed.
pub trait AsSchema {
    fn as_schema(&self) -> &Schema;
}

/// A committed schema, owned or borrowed.
pub trait AsCommitted: AsSchema {
    fn as_committed(&self) -> &Committed;
}

impl AsSchema for Schema {
    fn as_schema(&self) -> &Schema {
        self
    }
}

impl AsSchema for Committed {
    fn as_schema(&self) -> &Schema {
        self
    }
}

impl AsCommitted for Committed {
    fn as_committed(&self) -> &Committed {
        self
    }
}

impl<S: AsSchema> AsSchema for &S {
    fn as_schema(&self) -> &Schema {
        (**self).as_schema()
    }
}

impl<S: AsCommitted> AsCommitted for &S {
    fn as_committed(&self) -> &Committed {
        (**self).as_committed()
    }
}

/// A schema and the program checked against it.
#[derive(Debug)]
pub struct Compiled<S = Committed> {
    schema: Arc<S>,
    program: Arc<Program>,
}

impl<S> Clone for Compiled<S> {
    fn clone(&self) -> Self {
        Compiled {
            schema: Arc::clone(&self.schema),
            program: Arc::clone(&self.program),
        }
    }
}

impl<S: AsSchema> Compiled<S> {
    pub fn schema(&self) -> &Schema {
        self.schema.as_schema()
    }
}

impl<S> Compiled<S> {
    pub(super) fn new(schema: S, program: Program) -> Compiled<S> {
        Compiled {
            schema: Arc::new(schema),
            program: Arc::new(program),
        }
    }

    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Returns the schema shared with every clone of this pair.
    pub fn shared_schema(&self) -> &Arc<S> {
        &self.schema
    }

    /// Returns whether `other` is a clone of this pair.
    pub fn same(&self, other: &Compiled<S>) -> bool {
        Arc::ptr_eq(&self.schema, &other.schema) && Arc::ptr_eq(&self.program, &other.program)
    }
}

impl<S: AsCommitted> Compiled<S> {
    pub fn committed(&self) -> &Committed {
        self.schema.as_committed()
    }
}

impl Compiled {
    /// Returns the schema the terms of `a`, checked against this pair, name declarations of,
    /// with this pair's program.
    pub fn of_text(&self, a: &Adhoc) -> Compiled {
        match &a.temps {
            Some(t) => Compiled {
                schema: Arc::new(t.clone()),
                program: Arc::clone(&self.program),
            },
            None => self.clone(),
        }
    }
}

impl Compiled<Schema> {
    /// Returns the pair with its schema committed, `None` when [`Committed::new`] refuses the
    /// schema.
    pub fn commit(self) -> Option<Compiled> {
        let schema = Arc::try_unwrap(self.schema).unwrap_or_else(|s| (*s).clone());
        Some(Compiled {
            schema: Arc::new(Committed::new(schema)?),
            program: self.program,
        })
    }
}
