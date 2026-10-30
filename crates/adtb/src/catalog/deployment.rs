//! Deployment facts the roots heap holds beside the catalog's own: the durable owner of each
//! bundle family and each bundle's successful registration.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::heap::{self, FieldType, Heap};
use crate::key::{CtorId, Scalar, Value};
use crate::mvcc::{Scope, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::{Catalog, Error, ROOTS, int, uint, visible};

/// The constructor of the roots row marking a file whose families have durable owners.
const DEPLOYED: u32 = 11;
/// The constructor of a roots row naming a family's owner.
const FAMILY: u32 = 12;
/// The constructor of a roots row recording a bundle's successful registration.
const REGISTERED: u32 = 13;

/// Who registered a bundle: an open given it, or a role through the registration command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Open,
    Command,
}

/// A bundle's successful registration: the id of the role that registered it, how, and the
/// text of its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub role: i64,
    pub origin: Origin,
    pub text: String,
}

/// The deployment facts the latest commit holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deployed {
    /// Whether the file records family owners; a file written before it did has none.
    pub marked: bool,
    /// The id of each family's owner, by family.
    pub families: BTreeMap<String, i64>,
    /// The successful registration of each bundle, by bundle id.
    pub registered: BTreeMap<i64, Mark>,
}

/// One deployment fact to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact {
    Deployed,
    Family(String, i64),
    Registered(i64, Mark),
}

/// The writer of deployment facts into transactions of the write path.
#[derive(Clone)]
pub struct Deployment {
    roots: Arc<Heap>,
}

pub(super) fn ctors() -> [heap::CtorType; 3] {
    let ctor = |id: u32, fields: Vec<Scalar>| heap::CtorType {
        id: CtorId(id),
        fields: fields.into_iter().map(FieldType::Scalar).collect(),
    };
    [
        ctor(DEPLOYED, Vec::new()),
        ctor(FAMILY, vec![Scalar::Text, Scalar::I64]),
        ctor(
            REGISTERED,
            vec![Scalar::I64, Scalar::I64, Scalar::I64, Scalar::Text],
        ),
    ]
}

pub(super) fn holds(ctor: u32) -> bool {
    matches!(ctor, DEPLOYED | FAMILY | REGISTERED)
}

fn value(fact: &Fact) -> Value {
    let (ctor, fields) = match fact {
        Fact::Deployed => (DEPLOYED, Vec::new()),
        Fact::Family(family, role) => {
            (FAMILY, vec![Value::Text(family.clone()), Value::I64(*role)])
        }
        Fact::Registered(bundle, m) => (
            REGISTERED,
            vec![
                Value::I64(*bundle),
                Value::I64(m.role),
                int(match m.origin {
                    Origin::Open => 0,
                    Origin::Command => 1,
                }),
                Value::Text(m.text.clone()),
            ],
        ),
    };
    Value::Sum { ctor, fields }
}

impl Deployed {
    /// Hold `facts` as well, once they commit.
    pub fn record(&mut self, facts: &[Fact]) {
        for f in facts {
            match f {
                Fact::Deployed => self.marked = true,
                Fact::Family(family, role) => {
                    self.families.insert(family.clone(), *role);
                }
                Fact::Registered(bundle, m) => {
                    self.registered.insert(*bundle, m.clone());
                }
            }
        }
    }

    fn add(&mut self, ctor: u32, fields: &[Value]) -> Result<(), Error> {
        match (ctor, fields) {
            (DEPLOYED, []) => self.marked = true,
            (FAMILY, [Value::Text(family), Value::I64(role)]) => {
                self.families.insert(family.clone(), *role);
            }
            (
                REGISTERED,
                [
                    Value::I64(bundle),
                    Value::I64(role),
                    origin,
                    Value::Text(text),
                ],
            ) => {
                let origin = match uint(origin)? {
                    0 => Origin::Open,
                    1 => Origin::Command,
                    n => return Err(Error::Corrupt(format!("registration origin {n}"))),
                };
                self.registered.insert(
                    *bundle,
                    Mark {
                        role: *role,
                        origin,
                        text: text.clone(),
                    },
                );
            }
            _ => {
                return Err(Error::Corrupt(format!(
                    "a deployment row of constructor {ctor} holds {fields:?}"
                )));
            }
        }
        Ok(())
    }
}

impl Catalog {
    /// Returns the deployment facts the latest commit holds.
    pub fn deployed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Deployed, Error> {
        let (id, snapshot) = txns.take(Scope::Heaps([ROOTS].into()))?;
        let rows = visible(pager, txns, &snapshot, &self.roots);
        txns.release(id);
        let mut out = Deployed::default();
        for row in rows? {
            if let [Value::Sum { ctor, fields }] = row.as_slice()
                && holds(*ctor)
            {
                out.add(*ctor, fields)?;
            }
        }
        Ok(out)
    }

    /// Returns the writer of deployment facts.
    pub fn deployment(&self) -> Deployment {
        Deployment {
            roots: self.roots.clone(),
        }
    }

    pub(super) fn write_facts<R: Runtime>(
        &self,
        pager: &Pager<R>,
        x: Xid,
        facts: &[Fact],
    ) -> Result<(), Error> {
        self.deployment().insert(pager, x, facts)
    }
}

impl Deployment {
    /// Write `facts` in transaction `x`, so that they commit or abort with it.
    ///
    /// Precondition: `x` is a transaction of the write path over the pager this catalog opened.
    pub fn insert<R: Runtime>(
        &self,
        pager: &Pager<R>,
        x: Xid,
        facts: &[Fact],
    ) -> Result<(), Error> {
        for f in facts {
            self.roots.insert(pager, x.0, &[value(f)])?;
        }
        Ok(())
    }
}
