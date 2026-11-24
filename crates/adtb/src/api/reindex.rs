//! The `reindex` command: a job with its row of `sys::jobs`, on a thread of its own, that rebuilds
//! each named index in turn.

use std::sync::Weak;

use adtb_lang::ids::engine_rel;
use adtb_lang::schema::Schema;
use adtb_lang::types::{Fields, KeyCtors, RelId, RelRef, Ty};
use adtb_sys::lock::Lock;

use crate::catalog::{self, engine_relation};
use crate::exec::Val;
use crate::plan::IndexId;
use crate::runtime::{Runtime, ThreadId};
use crate::steps::Reindex;
use crate::write::{self, Failure, Found, Rebuild};

use super::Engine;
use super::text::{DynRows, rows};
use super::verify::{Job, Started};

/// What the running rebuild jobs need of the engine.
pub(super) struct Jobs<R: Runtime> {
    pub(super) me: Lock<Weak<Engine<R>>>,
    pub(super) threads: Lock<Vec<ThreadId>>,
    /// The jobs whose threads are about to return.
    finished: Lock<Vec<ThreadId>>,
}

impl<R: Runtime> Default for Jobs<R> {
    fn default() -> Self {
        Jobs {
            me: Lock::new(Weak::new()),
            threads: Lock::default(),
            finished: Lock::default(),
        }
    }
}

/// What one job does.
struct Work {
    rel: RelId,
    indexes: Vec<IndexId>,
    online: bool,
    run: i64,
    target: String,
    table: RelId,
}

impl<R: Runtime> Engine<R> {
    /// Start a job rebuilding the indexes of `rel` at `positions`, or every one. Returns its key.
    pub(super) fn reindex_job(
        &self,
        schema: &Schema,
        rel: RelId,
        positions: Option<&[usize]>,
        online: bool,
    ) -> Result<DynRows, write::Error> {
        self.join_finished();
        self.db.admit_reindex(rel)?;
        let indexes = self.db.reindexed_ids(rel, positions)?;
        let table = engine_relation(schema, engine_rel::JOBS);
        let me = self.reindexing.me.lock(&self.rt).clone();
        let target = schema.relation(rel).name.clone();
        let started = self.reindex_row(schema, table, target.clone()).hand_over();
        let run = started.run;
        let work = Work {
            rel,
            indexes,
            online,
            run,
            target,
            table,
        };
        let thread = self.rt.spawn("reindex", move || {
            if let Some(me) = me.upgrade() {
                me.run_reindex(&work, started);
                let thread = me.rt.current();
                me.reindexing.finished.lock(&me.rt).push(thread);
            }
        });
        self.reindexing.threads.lock(&self.rt).push(thread);
        let ty = Ty::Record(Fields::new([(
            "job".into(),
            Ty::Key {
                rel: RelRef::Declared(table),
                ctors: KeyCtors::All,
            },
        )]));
        Ok(rows(schema, ty, vec![Val::record(vec![Val::Int(run)])]))
    }

    /// Stop the running jobs and wait for them, unless called from one.
    pub(super) fn stop_reindexing(&self) {
        self.db.stop_rebuilds();
        let me = self.rt.current();
        let threads = std::mem::take(&mut *self.reindexing.threads.lock(&self.rt));
        for t in threads {
            if t != me {
                self.rt.join(t);
            }
        }
    }

    /// Join the threads of the jobs that have ended.
    fn join_finished(&self) {
        let finished = std::mem::take(&mut *self.reindexing.finished.lock(&self.rt));
        for t in finished {
            let mut threads = self.reindexing.threads.lock(&self.rt);
            let Some(at) = threads.iter().position(|x| *x == t) else {
                continue;
            };
            threads.swap_remove(at);
            drop(threads);
            self.rt.join(t);
        }
    }

    fn run_reindex(&self, work: &Work, started: Started) {
        let schema = self.schema().clone();
        let schema = schema.as_ref();
        let job = self.reindex_row_of(schema, work.table, started);
        self.rebuild_all(schema, work, &job);
    }

    fn rebuild_all(&self, schema: &Schema, work: &Work, job: &Job<'_, R>) {
        let mapping = self.db.mapping();
        let grant = match self.db.rebuild_grant(&mapping) {
            Ok(g) => g,
            Err(cause) => {
                if let Some(c) = cause {
                    let at = format!("reindex of {}: {c}", work.target);
                    for &id in &work.indexes {
                        self.keep_outcome(schema, id, work.run, Some(at.clone()));
                    }
                }
                return;
            }
        };
        let no_vacuum = self.db.holding_off_vacuum();
        let phase = |p: Reindex| job.phase(p.name(), p.done());
        let marked = match self.catalog.begin_reindexing(&self.pager, self.db.txns()) {
            Ok(marked) => marked,
            Err(e) => {
                let at = format!("reindex of {}: {}", work.target, catalog::detail(&e));
                for &id in &work.indexes {
                    self.keep_outcome(schema, id, work.run, Some(at.clone()));
                }
                return;
            }
        };
        let mut reclaimed = marked.is_some();
        for &index in &work.indexes {
            let rebuild = Rebuild {
                rel: work.rel,
                index,
                online: work.online,
                phase: &phase,
            };
            match no_vacuum.rebuild(&rebuild, &grant, &mapping) {
                Ok(all_freed) => {
                    reclaimed &= all_freed;
                    self.keep_outcome(schema, index, work.run, None);
                }
                Err(Failure {
                    cause,
                    reclaimed: r,
                }) => {
                    reclaimed &= r;
                    if let Some(cause) = cause {
                        let name = self.db.index_clause(work.rel, index);
                        let at = format!("reindex of {name}: {cause}");
                        self.keep_outcome(schema, index, work.run, Some(at));
                    }
                }
            }
        }
        if let Some(marked) = marked.filter(|_| reclaimed)
            && let Err(e) = self.catalog.end_reindexing(&self.pager, marked)
        {
            self.keep_outcome(
                schema,
                work.indexes[0],
                work.run,
                Some(format!(
                    "reindex of {}: {}",
                    work.target,
                    catalog::detail(&e)
                )),
            );
        }
    }

    /// Replace the findings of the last rebuild of `index` with one saying what `failed` says.
    fn keep_outcome(&self, schema: &Schema, index: IndexId, run: i64, failed: Option<String>) {
        let found = failed
            .map(|at| Found::Structure { at })
            .into_iter()
            .collect();
        self.keep_reindex_findings(schema, index.0, run, found);
    }
}
