use adtb_lang::kernel::{Build, Terms};
use adtb_lang::types::{RelId, Ty};

use std::sync::Arc;

use crate::exec::{Grant, Val};
use crate::runtime::Runtime;
use crate::write::{Bounds, Local, Text, Txn, either};

use super::super::{Engine, Error, TxnId};

#[derive(Default)]
pub(super) struct Changes {
    rows: Vec<(RelId, bool, Val)>,
}

impl Changes {
    pub(super) fn insert(&mut self, relation: RelId, row: Val) {
        self.rows.push((relation, true, row));
    }

    pub(super) fn delete(&mut self, relation: RelId, row: Val) {
        self.rows.push((relation, false, row));
    }

    /// Apply the rows in `tx`, charging the statement to `grant`, or to a grant of its own when
    /// `None`.
    pub(super) fn apply<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        grant: Option<&Arc<Grant>>,
    ) -> Result<(), crate::write::Error> {
        if self.rows.is_empty() {
            return Ok(());
        }
        let mut terms = Terms::default();
        let mut params = Vec::new();
        let mut changes = Vec::new();
        for (rel, insert, row) in &self.rows {
            let ty = Ty::Set(Box::new(Ty::Data {
                def: e.schema().relation(*rel).elem_def,
                args: Vec::new(),
                ctors: None,
            }));
            let var = terms.var("row·", ty.clone());
            params.push((var, Val::set([row.clone()].into())));
            let mut b = Build::new(e.schema(), &mut terms);
            let rows = b.var(var);
            changes.push(if *insert {
                b.insert(*rel, rows)
            } else {
                b.delete(*rel, rows)
            });
        }
        let change = Build::new(e.schema(), &mut terms).par(changes);
        let text = Text {
            schema: e.schema(),
            terms: &terms,
            items: &[change],
            params: &params,
            clock: e.clock(),
            caller: None,
        };
        let bounds = Bounds {
            grant: grant.cloned(),
            ..Bounds::default()
        };
        e.db.statements(tx, text, &mut Local::default(), bounds)
            .map(|_| ())
            .map_err(|failure| failure.error)
    }
}

pub(super) fn run<R: Runtime, T>(
    e: &Engine<R>,
    write: bool,
    body: &mut impl FnMut(&Engine<R>, &Txn, TxnId, &mut Changes) -> Result<T, Error>,
) -> Result<T, Error> {
    run_under(e, write, None, body)
}

/// Run `body` as [`run`] does, its changes charged to `grant`.
pub(super) fn run_under<R: Runtime, T>(
    e: &Engine<R>,
    write: bool,
    grant: Option<&Arc<Grant>>,
    body: &mut impl FnMut(&Engine<R>, &Txn, TxnId, &mut Changes) -> Result<T, Error>,
) -> Result<T, Error> {
    let retries = e.settings.lock(&e.rt).retries;
    let mut attempt = 0;
    loop {
        let id = e.txn_id();
        let tx = if write {
            e.begin(id)?
        } else {
            e.begin_read(id, false)?
        };
        let mut changes = Changes::default();
        let result = body(e, &tx, id, &mut changes).and_then(|value| {
            changes
                .apply(e, &tx, grant)
                .map_err(|error| e.write_error(id, error))?;
            Ok(value)
        });
        let result = match result {
            Ok(value) => {
                e.db.commit(tx)
                    .map(|_| value)
                    .map_err(|error| e.write_error(id, error))
            }
            Err(error) => Err(either(e.abandon(id, tx, || error))),
        };
        match result {
            Err(error) if error.is_serialization() && attempt < retries => attempt += 1,
            done => return done,
        }
    }
}
