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

    /// Returns this pair with its schema borrowed.
    pub(crate) fn viewed(&self) -> Compiled<&Schema> {
        self.sharing(self.schema())
    }

    /// Returns `schema` with this pair's program, refused naming the first object of this pair's
    /// schema with an id that `schema` holds at another position or not at all.
    pub(crate) fn extended<T: AsSchema>(&self, schema: T) -> Result<Compiled<T>, String> {
        let was = self.schema();
        match was.first_moved(schema.as_schema()) {
            Some(moved) => Err(format!(
                "the {} the program names moved in the schema it is paired with",
                moved.named(was)
            )),
            None => Ok(self.sharing(schema)),
        }
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

    /// Returns `schema` paired with this pair's program, unchecked.
    pub(super) fn sharing<T>(&self, schema: T) -> Compiled<T> {
        Compiled {
            schema: Arc::new(schema),
            program: Arc::clone(&self.program),
        }
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

impl<S: AsCommitted> Compiled<S> {
    /// Returns the schema the terms of `a`, checked against this pair, name declarations of,
    /// with this pair's program, both borrowed.
    pub(crate) fn of_adhoc<'a>(&'a self, a: &'a Adhoc) -> Compiled<&'a Committed> {
        self.sharing(a.temps.as_ref().unwrap_or(self.committed()))
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
