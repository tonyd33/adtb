use std::fmt::Display;
use std::io::{self, Write};

use adtb::{AdHocTransaction, CallOptions, Database, TxnCallOptions, TxnOptions};
use adtb_lang::textual::print;

/// A line the shell acts on itself rather than passing to the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Quit,
    Begin,
    Commit,
    Rollback,
}

impl Command {
    /// Returns the command `line` names, or `None` when it is ad hoc text.
    pub fn of(line: &str) -> Option<Command> {
        match line.trim() {
            "\\q" => Some(Command::Quit),
            "\\begin" => Some(Command::Begin),
            "\\commit" => Some(Command::Commit),
            "\\rollback" => Some(Command::Rollback),
            _ => None,
        }
    }
}

/// An open database, the interactive transaction a `\begin` opened on it, and where results
/// and errors go.
pub struct Session<'db, O, E> {
    db: &'db Database,
    txn: Option<AdHocTransaction<'db>>,
    out: O,
    err: E,
    failed: bool,
}

impl<'db, O: Write, E: Write> Session<'db, O, E> {
    pub fn new(db: &'db Database, out: O, err: E) -> Session<'db, O, E> {
        Session {
            db,
            txn: None,
            out,
            err,
            failed: false,
        }
    }

    /// Returns whether any text or command has failed.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Returns where results go, to write a prompt there.
    pub fn out(&mut self) -> &mut O {
        &mut self.out
    }

    /// Run `text` in the open transaction, or as a transaction of its own when none is open,
    /// and print each row of its results on a line.
    pub fn run(&mut self, text: &str) -> io::Result<()> {
        let ran = match &mut self.txn {
            Some(t) => t.query_text(text, &[], &TxnCallOptions::default()),
            None => self.db.query_text(text, &[], &CallOptions::default()),
        };
        let results = match ran {
            Ok(results) => results,
            Err(e) => return self.fail(e),
        };
        for rows in results {
            let form = rows.form().clone();
            for row in rows {
                let printed = match row {
                    Ok(v) => print(&v, &form),
                    Err(e) => return self.fail(e),
                };
                match printed {
                    Ok(line) => writeln!(self.out, "{line}")?,
                    Err(e) => return self.fail(e),
                }
            }
        }
        self.out.flush()
    }

    /// Act on `c`. Returns whether the shell reads on.
    pub fn command(&mut self, c: Command) -> io::Result<bool> {
        let done = match (c, self.txn.take()) {
            (Command::Quit, txn) => {
                self.txn = txn;
                return Ok(false);
            }
            (Command::Begin, Some(t)) => {
                self.txn = Some(t);
                return self.fail("a transaction is already open").map(|()| true);
            }
            (Command::Commit | Command::Rollback, None) => {
                return self.fail("no transaction is open").map(|()| true);
            }
            (Command::Begin, None) => self
                .db
                .begin(&TxnOptions::default())
                .map(|t| self.txn = Some(t)),
            (Command::Commit, Some(t)) => t.commit().map(drop),
            (Command::Rollback, Some(t)) => t.rollback(),
        };
        if let Err(e) = done {
            self.fail(e)?;
        }
        Ok(true)
    }

    /// Roll back the open transaction, if any, saying so.
    pub fn finish(&mut self) -> io::Result<()> {
        match self.txn.take() {
            None => Ok(()),
            Some(t) => {
                writeln!(self.err, "the open transaction is rolled back")?;
                t.rollback().or_else(|e| self.fail(e))
            }
        }
    }

    fn fail(&mut self, e: impl Display) -> io::Result<()> {
        self.failed = true;
        writeln!(self.err, "{e}")?;
        self.err.flush()
    }
}
