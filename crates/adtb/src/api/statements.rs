use crate::mvcc::Xid;
use crate::runtime::Runtime;
use crate::write::{self, Bounds, Local, Text, Txn};

use super::text::{DynRows, Prepared};
use super::{Engine, Error, TxnId};

/// Why [`Engine::resumable`] stopped short.
pub(super) enum Halted {
    /// A statement must wait for transaction `on` to end; a rerun passes `rerun`.
    Blocked {
        on: Xid,
        rerun: write::Rerun,
    },
    Failed(Failure),
}

/// How a statement or commit of an attempt failed.
pub(super) struct Failure {
    /// The client error.
    pub(super) error: Error,
    /// Whether it ends the transaction.
    pub(super) ends: bool,
    /// Whether a command of the attempt has finished. A refused or failed one has not.
    ///
    /// Precondition on a command: one that fails with a serialization failure has taken no effect.
    pub(super) commanded: bool,
}

impl<R: Runtime> Engine<R> {
    /// Run the items of `p` as [`statements`](Engine::statements) does, except that a
    /// statement suspending on a transaction wait halts with `Blocked`. A command after the first,
    /// and an item after a command, polls with its stops instead.
    pub(super) fn resumable(
        &self,
        id: TxnId,
        tx: &Txn,
        p: &Prepared,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<Vec<DynRows>, Halted> {
        let schema = p.schema();
        let text = Text {
            schema,
            terms: p.terms(),
            items: p.items(),
            params: p.params(),
            clock: self.clock(),
            caller: Some(p.caller()),
        };
        let items = Bounds {
            polls_from: p.commands().iter().map(|c| c.at).min(),
            ..bounds.clone()
        };
        let mut ran: Vec<(usize, DynRows)> = Vec::new();
        let mut refused: Option<Error> = None;
        let mut between = |at: usize, local: &mut Local| -> Result<(), write::Error> {
            let commands = p.commands().iter().enumerate();
            for (nth, c) in commands.filter(|(_, c)| c.at == at) {
                match self.deployment_command(&c.command, p) {
                    Some(Ok(rows)) => ran.push((at, rows)),
                    Some(Err(e)) => {
                        refused = Some(e);
                        return Err(write::Error::Internal(
                            "a deployment command was refused".into(),
                        ));
                    }
                    None => {
                        if !matches!(c.command, adtb_lang::check::Command::Verify(_)) {
                            self.db.admit_database()?;
                        }
                        let c = (&c.command, at);
                        let bounds = if nth == 0 {
                            bounds.clone()
                        } else {
                            Bounds {
                                suspend: false,
                                rerun: None,
                                ..bounds.clone()
                            }
                        };
                        ran.push((at, self.command(tx, p, c, local, bounds)?));
                    }
                }
            }
            Ok(())
        };
        let done = self
            .db
            .statements_with(tx, text, local, items, &mut between);
        let commanded = !ran.is_empty();
        let failed = |error, ends| {
            Halted::Failed(Failure {
                error,
                ends,
                commanded,
            })
        };
        match (done, refused) {
            (Err(_), Some(e)) => Err(failed(e, false)),
            (Ok(outcomes), _) => p
                .results(schema, outcomes, ran)
                .map_err(|e| failed(e, true)),
            (
                Err(write::Failed {
                    error: write::Error::Blocked { on, rerun },
                    ..
                }),
                None,
            ) => Err(Halted::Blocked { on, rerun }),
            (Err(f), None) => {
                let ends = f.error.ends_transaction();
                let error = self.db.quarantining(f.error);
                Err(failed(Error::of_write(schema, id, error), ends))
            }
        }
    }
}
