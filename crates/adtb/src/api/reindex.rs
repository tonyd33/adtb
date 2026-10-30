//! The `reindex` command: a job with its row of `sys::jobs`, on a thread of its own, that rebuilds
//! each named index in turn.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Arc, Weak};

use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty};
use adtb_sys::lock::Lock;

use crate::exec::Val;
use crate::plan::IndexId;
use crate::runtime::{Runtime, ThreadId};
use crate::write::{self, Failure, Found, Rebuild, panic_text};

use super::Engine;
use super::command::engine_relation;
use super::text::{DynRows, rows};
use super::verify::{Job, Started};

/// Called with each phase of a rebuild as it starts, on the job's thread.
pub type PhaseHook = Arc<dyn Fn(&str) + Send + Sync>;

/// What the running rebuild jobs need of the engine.
pub(super) struct Jobs<R: Runtime> {
    pub(super) me: Lock<Weak<Engine<R>>>,
    pub(super) threads: Lock<Vec<ThreadId>>,
    /// The jobs whose threads are about to return.
    finished: Lock<Vec<ThreadId>>,
    pub(super) hook: Lock<Option<PhaseHook>>,
}

impl<R: Runtime> Default for Jobs<R> {
    fn default() -> Self {
        Jobs {
            me: Lock::new(Weak::new()),
            threads: Lock::default(),
            finished: Lock::default(),
            hook: Lock::new(None),
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
        let table = engine_relation(schema, "sys", "jobs")?;
        let me = self.reindexing.me.lock(&self.rt).clone();
        let target = schema.relation(rel).name.clone();
        let started = self.reindex_row(table, target.clone()).hand_over();
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
        let ty = Ty::Record(vec![(
            "job".into(),
            Ty::Key {
                rel: table,
                ctors: None,
            },
        )]);
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
        let schema = self.schema.clone();
        let schema = schema.as_ref();
        let job = self.reindex_row_of(work.table, started);
        let ran = catch_unwind(AssertUnwindSafe(|| self.rebuild_all(schema, work, &job)));
        drop(job);
        if let Err(payload) = ran {
            let Some(text) = panic_text(&*payload) else {
                resume_unwind(payload);
            };
            let at = format!("reindex of {}: {text}", work.target);
            for &id in &work.indexes {
                let found = BTreeSet::from([Found::Structure { at: at.clone() }]);
                if let Err(e) = self.keep_reindex_findings(schema, id.0, work.run, found) {
                    panic!("the engine declares `sys::verify_findings`: {e:?}");
                }
            }
        }
    }

    fn rebuild_all(&self, schema: &Schema, work: &Work, job: &Job<'_, R>) {
        let _no_vacuum = self.db.holding_off_vacuum();
        let hook = self.reindexing.hook.lock(&self.rt).clone();
        let phase = |name: &str, done: f64| {
            job.phase(name, done);
            if let Some(hook) = &hook {
                hook(name);
            }
        };
        let was_marked = match self.catalog.begin_reindexing(&self.pager) {
            Ok(marked) => marked,
            Err(e) => {
                let at = format!("reindex of {}: {e:?}", work.target);
                for &id in &work.indexes {
                    self.keep_outcome(schema, id, work.run, Some(at.clone()));
                }
                return;
            }
        };
        let mut reclaimed = !was_marked;
        for &index in &work.indexes {
            let rebuild = Rebuild {
                rel: work.rel,
                index,
                online: work.online,
                phase: &phase,
            };
            match self.db.rebuild(&rebuild) {
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
        if reclaimed && let Err(e) = self.catalog.end_reindexing(&self.pager) {
            self.keep_outcome(
                schema,
                work.indexes[0],
                work.run,
                Some(format!("reindex of {}: {e:?}", work.target)),
            );
        }
    }

    /// Replace the findings of the last rebuild of `index` with one saying what `failed` says.
    fn keep_outcome(&self, schema: &Schema, index: IndexId, run: i64, failed: Option<String>) {
        let found = failed
            .map(|at| Found::Structure { at })
            .into_iter()
            .collect();
        if let Err(e) = self.keep_reindex_findings(schema, index.0, run, found) {
            panic!("the engine declares `sys::verify_findings`: {e:?}");
        }
    }
}
