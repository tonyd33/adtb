use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use crate::runtime::Runtime;

use super::Error;
use super::code::Catch;
use super::frame::{Frame, Sel};
use super::ir::{Build, Chain, Step};
use super::memory::Arena;
use super::run::{Ctx, Expand, Run};
use super::spill::{Distinct, Records, Stream, Writer};
use super::value::Val;

/// The partitions of a keyed build too large for memory.
const PARTS: usize = 16;

/// A build's elements by key.
pub enum Table {
    Mem {
        rows: HashMap<Vec<Val>, Vec<Val>>,
        _arena: Arena,
    },
    /// Records of a key and an element, partitioned by the key's hash; one partition when the
    /// build has no key.
    Parts(Vec<Stream>),
}

/// Rows that reached a probe of a partitioned build, waiting for their partitions.
pub struct Deferral {
    chain: Arc<Chain>,
    step: usize,
    build: usize,
    /// The variables bound to one value at every row, the same for every row deferred at this
    /// probe.
    fixed: Vec<(u32, Val)>,
    /// Per partition, records of a row's key and its varying variables.
    parts: Vec<Writer>,
}

/// The deferrals of a query's evaluation, by chain and step.
pub type Deferred = BTreeMap<(usize, usize), Deferral>;

/// The rows of one probe waiting for one partition.
struct Waiting {
    chain: Arc<Chain>,
    step: usize,
    fixed: Vec<(u32, Val)>,
    rows: Stream,
}

/// Rows that reach a probe: a frame, its selected rows, each one's key values, and the variable
/// each match binds.
pub struct Probed<'a> {
    pub f: &'a Frame,
    pub sel: &'a Sel,
    pub keys: &'a HashMap<u32, Vec<Val>>,
    pub var: u32,
}

/// The part of a partitioned build's elements loaded at once.
struct Chunk {
    rows: HashMap<Vec<Val>, Vec<Val>>,
    _arena: Arena,
}

fn part(key: &Val, parts: usize) -> usize {
    if parts == 1 {
        return 0;
    }
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    (h.finish() % parts as u64) as usize
}

fn pair(mut rec: Vec<Val>) -> Result<(Val, Val), Error> {
    match (rec.pop(), rec.pop(), rec.is_empty()) {
        (Some(b), Some(a), true) => Ok((a, b)),
        _ => Err(Error::Corrupt(
            "a spilled build record is not a pair".into(),
        )),
    }
}

impl<R: Runtime> Ctx<R> {
    /// Returns the table of build `i` of `run`'s query, made on first use.
    pub(super) fn table(self: &Arc<Self>, run: &Run, i: usize) -> Result<Arc<Table>, Error> {
        let mut slot = run.builds[i].lock(&self.rt);
        if let Some(t) = &*slot {
            return Ok(t.clone());
        }
        let build: &Build = &run.query.builds[i];
        let mut out = Distinct::new(&self.grant)?;
        for p in &build.pipelines {
            self.pipeline(run, p, &run.env, &mut out)?;
        }
        let t = Arc::new(self.hash(build, &run.env, out)?);
        *slot = Some(t.clone());
        Ok(t)
    }

    /// Returns the table of `elems` keyed by `build`'s key, in memory while it holds at most
    /// a quarter of the grant and partitioned in the spill file past that.
    fn hash(&self, build: &Build, env: &Frame, elems: Distinct) -> Result<Table, Error> {
        let parts = if build.key.is_empty() { 1 } else { PARTS };
        let cap = self.grant.limit() / 4;
        let mut rows: HashMap<Vec<Val>, Vec<Val>> = HashMap::new();
        let mut arena = self.grant.take(0)?;
        let mut spilled: Option<Vec<Writer>> = None;
        let mut m = elems.merge(&self.spill)?;
        loop {
            let chunk = m.chunk(self.options.morsel_rows)?;
            if chunk.is_empty() {
                break;
            }
            let mut f = env.widen(chunk.len());
            f.bind_all(build.var, chunk.clone());
            let mut sel = f.all();
            let mut keys = self.eval_keys(&build.key, &mut f, &mut sel, &mut Catch::off())?;
            for (r, e) in chunk.into_iter().enumerate() {
                let k = keys.remove(&(r as u32)).expect("a key per element");
                if spilled.is_none() {
                    let n = k.iter().map(Val::size).sum::<usize>() + e.size();
                    if arena.bytes() + n <= cap && arena.grow(n).is_ok() {
                        rows.entry(k).or_default().push(e);
                        continue;
                    }
                    let mut ws: Vec<Writer> = (0..parts).map(|_| Writer::default()).collect();
                    for (k, es) in rows.drain() {
                        let k = Val::record(k);
                        for e in es {
                            ws[part(&k, parts)].put(&self.spill, &[&k, &e])?;
                        }
                    }
                    arena.release(usize::MAX);
                    spilled = Some(ws);
                }
                let ws = spilled.as_mut().expect("a build spills once");
                let k = Val::record(k);
                ws[part(&k, parts)].put(&self.spill, &[&k, &e])?;
            }
        }
        match spilled {
            None => Ok(Table::Mem {
                rows,
                _arena: arena,
            }),
            Some(ws) => Ok(Table::Parts(
                ws.into_iter()
                    .map(|w| w.finish(&self.spill))
                    .collect::<Result<_, _>>()?,
            )),
        }
    }

    /// Pass each row of `sel`, once per element of `table` its key matches, on from the probe
    /// at step `at` of `c`; a row of a partitioned table waits for its partition.
    pub(super) fn probe(
        self: &Arc<Self>,
        run: &Run,
        c: &Arc<Chain>,
        at: usize,
        table: &Table,
        input: &Probed<'_>,
        out: &mut Distinct,
    ) -> Result<(), Error> {
        let Probed { f, sel, keys, .. } = *input;
        match table {
            Table::Mem { rows, .. } => self.matches(run, c, at + 1, rows, input, out),
            Table::Parts(parts) => {
                let Step::Probe { build, .. } = c.steps[at] else {
                    unreachable!("a probe defers at a probe step")
                };
                let mut d = run.deferred.lock(&self.rt);
                let w = d
                    .entry((Arc::as_ptr(c) as usize, at))
                    .or_insert_with(|| Deferral {
                        chain: c.clone(),
                        step: at,
                        build,
                        fixed: f.fixed(),
                        parts: (0..parts.len()).map(|_| Writer::default()).collect(),
                    });
                for &r in sel {
                    let k = Val::record(keys[&r].clone());
                    let vs: Vec<Val> = f
                        .varying(r)
                        .into_iter()
                        .flat_map(|(v, x)| [Val::Int(i64::from(v)), x])
                        .collect();
                    w.parts[part(&k, parts.len())].put(&self.spill, &[&k, &Val::record(vs)])?;
                }
                Ok(())
            }
        }
    }

    fn matches(
        self: &Arc<Self>,
        run: &Run,
        c: &Arc<Chain>,
        next: usize,
        rows: &HashMap<Vec<Val>, Vec<Val>>,
        input: &Probed<'_>,
        out: &mut Distinct,
    ) -> Result<(), Error> {
        let Probed { f, sel, keys, var } = *input;
        let mut found = Expand::new(f, var, self.options.morsel_rows);
        for &r in sel {
            let k = &keys[&r];
            let hits: Box<dyn Iterator<Item = &Val>> = if k.is_empty() {
                Box::new(rows.values().flatten())
            } else {
                Box::new(rows.get(k).into_iter().flatten())
            };
            for e in hits {
                if let Some((g, s)) = found.push(r, e.clone()) {
                    self.chain(run, c, next, g, s, out)?;
                }
            }
        }
        if let Some((g, s)) = found.rest() {
            self.chain(run, c, next, g, s, out)?;
        }
        Ok(())
    }

    /// Pass every deferred row on against its partition, a chunk of the partition at a time,
    /// until no row waits.
    pub(super) fn drain(self: &Arc<Self>, run: &Run, out: &mut Distinct) -> Result<(), Error> {
        loop {
            let deferred = std::mem::take(&mut *run.deferred.lock(&self.rt));
            if deferred.is_empty() || run.stopped() {
                return Ok(());
            }
            let mut by: BTreeMap<(usize, usize), Vec<Waiting>> = BTreeMap::new();
            for d in deferred.into_values() {
                for (p, w) in d.parts.into_iter().enumerate() {
                    let rows = w.finish(&self.spill)?;
                    if !rows.is_empty() {
                        by.entry((d.build, p)).or_default().push(Waiting {
                            chain: d.chain.clone(),
                            step: d.step,
                            fixed: d.fixed.clone(),
                            rows,
                        });
                    }
                }
            }
            for ((b, p), waiting) in by {
                self.check_cancel()?;
                let table = run.builds[b]
                    .lock(&self.rt)
                    .clone()
                    .expect("a deferred probe's build is made");
                let Table::Parts(parts) = &*table else {
                    unreachable!("only a partitioned build defers its probes")
                };
                let mut elems = Records::new(&self.spill, &parts[p]);
                let mut next = None;
                loop {
                    let (chunk, more) = self.load(&mut elems, &mut next)?;
                    for w in &waiting {
                        self.replay(run, w, &chunk.rows, out)?;
                    }
                    if !more {
                        break;
                    }
                }
            }
        }
    }

    /// Returns the next elements of a partition that fit half the grant's room, and whether
    /// more remain. `next` carries a record read past the chunk to the next call.
    fn load(
        &self,
        elems: &mut Records<'_, R>,
        next: &mut Option<Vec<Val>>,
    ) -> Result<(Chunk, bool), Error> {
        let cap = self.grant.limit().saturating_sub(self.grant.used()) / 2;
        let mut arena = self.grant.take(0)?;
        let mut rows: HashMap<Vec<Val>, Vec<Val>> = HashMap::new();
        let more = loop {
            let Some(rec) = next.take().map_or_else(|| elems.read(), |r| Ok(Some(r)))? else {
                break false;
            };
            let n = rec.iter().map(Val::size).sum::<usize>();
            if !rows.is_empty() && arena.bytes() + n > cap {
                *next = Some(rec);
                break true;
            }
            arena.grow(n)?;
            let (k, e) = pair(rec)?;
            rows.entry(k.fields()?.to_vec()).or_default().push(e);
        };
        Ok((
            Chunk {
                rows,
                _arena: arena,
            },
            more,
        ))
    }

    /// Pass the rows of `w` on against the elements `rows` of their partition, a morsel at a time.
    fn replay(
        self: &Arc<Self>,
        run: &Run,
        w: &Waiting,
        rows: &HashMap<Vec<Val>, Vec<Val>>,
        out: &mut Distinct,
    ) -> Result<(), Error> {
        let Step::Probe { var, .. } = &w.chain.steps[w.step] else {
            unreachable!("a deferral waits at a probe")
        };
        let mut recs = Records::new(&self.spill, &w.rows);
        loop {
            let mut keys = HashMap::new();
            let mut bound = Vec::new();
            while bound.len() < self.options.morsel_rows.max(1) {
                let Some(rec) = recs.read()? else { break };
                let (k, vs) = pair(rec)?;
                keys.insert(bound.len() as u32, k.fields()?.to_vec());
                let row = vs
                    .fields()?
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|[v, x]| Ok((u32::try_from(v.int()?).map_err(|_| bad_row())?, x.clone())))
                    .collect::<Result<Vec<_>, Error>>()?;
                bound.push(row);
            }
            if bound.is_empty() {
                return Ok(());
            }
            let f = Frame::rebuild(self.program.vars, &w.fixed, bound);
            let _held = self.grant.take(f.size())?;
            let sel = f.all();
            let input = Probed {
                f: &f,
                sel: &sel,
                keys: &keys,
                var: *var,
            };
            self.matches(run, &w.chain, w.step + 1, rows, &input, out)?;
        }
    }
}

fn bad_row() -> Error {
    Error::Corrupt("a deferred row names no variable".into())
}
