use std::collections::BTreeMap;

use adtb_lang::schema::{Generator, Schema};
use adtb_lang::types::RelId;

use crate::btree::Index;
use crate::key::Value;
use crate::runtime::Runtime;
use crate::storage::{self, HEADER_LEN, PageError, PageKind, Pager, Pid};

use super::Error;

const FORMAT: u16 = 0x5351;

/// The values each counter draws past its reservation before it reserves again.
const BLOCK: i64 = 64;

/// The id of the counter of `load` values.
pub const LOADS: u32 = u32::MAX;

/// A counter's next candidate and the exclusive end of its durable reservation.
/// The maximum candidate marks exhaustion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Counter {
    next: i64,
    reserved: i64,
}

/// The counters a database draws generated values from, each reserved durably in blocks.
///
/// Page layout after the header: a `u32` count, then each counter's `u32` id and the `i64` its
/// reservation ends before, in id order.
///
/// Postcondition: a value is never drawn twice, across any number of crashes.
#[derive(Debug)]
pub struct Sequences {
    pid: Pid,
    counters: BTreeMap<u32, Counter>,
}

impl Sequences {
    /// Allocate an empty page of counters. Returns its page id.
    pub fn create<R: Runtime>(pager: &Pager<R>) -> Result<Pid, PageError> {
        let pid = pager.allocate(FORMAT, PageKind::Catalog, 0)?;
        pager.update(&[pid], |p| p[0][HEADER_LEN..HEADER_LEN + 4].fill(0))?;
        pager.flush_log();
        Ok(pid)
    }

    /// Returns no counters, for a database that runs no write.
    pub fn none() -> Sequences {
        Sequences {
            pid: Pid(0),
            counters: BTreeMap::new(),
        }
    }

    /// Read the counters of page `pid`. Each counter next draws its reservation's end.
    ///
    /// Panics if `pid` is not a page of counters.
    pub fn open<R: Runtime>(pager: &Pager<R>, pid: Pid) -> Result<Sequences, PageError> {
        let page = pager.read(pid)?;
        assert_eq!(
            (PageKind::Catalog as u16, FORMAT),
            (storage::kind(&page), storage::format(&page)),
            "{pid:?} is a page of counters"
        );
        let body = &page[HEADER_LEN..];
        let n = u32::from_le_bytes(body[..4].try_into().expect("4 bytes")) as usize;
        let mut counters = BTreeMap::new();
        for i in 0..n {
            let at = 4 + i * 12;
            let id = u32::from_le_bytes(body[at..at + 4].try_into().expect("4 bytes"));
            let reserved = i64::from_le_bytes(body[at + 4..at + 12].try_into().expect("8 bytes"));
            counters.insert(
                id,
                Counter {
                    next: reserved,
                    reserved,
                },
            );
        }
        Ok(Sequences { pid, counters })
    }

    pub fn root(&self) -> Pid {
        self.pid
    }

    /// Returns the next value of counter `id`, at least `floor`.
    pub fn draw<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        id: u32,
        floor: i64,
    ) -> Result<i64, Error> {
        let mut c = self.counter(id);
        let v = c.next.max(floor);
        c.next = v.checked_add(1).ok_or(Error::SequenceExhausted)?;
        if c.next > c.reserved {
            c.reserved = v.saturating_add(BLOCK);
        }
        self.replace(pager, id, c)?;
        Ok(v)
    }

    /// Advance counter `id` past `held`, durably.
    /// Keep the maximum bound as exhaustion for a maximum explicit key.
    pub fn advance<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        id: u32,
        held: i64,
    ) -> Result<(), Error> {
        let mut c = self.counter(id);
        c.next = c.next.max(held.saturating_add(1));
        c.reserved = c.reserved.max(c.next);
        self.replace(pager, id, c)
    }

    fn counter(&self, id: u32) -> Counter {
        self.counters.get(&id).copied().unwrap_or(Counter {
            next: 1,
            reserved: 1,
        })
    }

    fn replace<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        id: u32,
        counter: Counter,
    ) -> Result<(), Error> {
        let previous = self.counters.insert(id, counter);
        if previous.is_none_or(|old| old.reserved != counter.reserved)
            && let Err(error) = self.persist(pager)
        {
            match previous {
                Some(old) => {
                    self.counters.insert(id, old);
                }
                None => {
                    self.counters.remove(&id);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// Write every reservation to the page and force the log.
    fn persist<R: Runtime>(&self, pager: &Pager<R>) -> Result<(), Error> {
        let room = (pager.page_size() - HEADER_LEN - 4) / 12;
        if self.counters.len() > room {
            return Err(Error::Unsupported(format!(
                "more than {room} generated counters"
            )));
        }
        let mut body = Vec::with_capacity(4 + 12 * self.counters.len());
        body.extend_from_slice(&(self.counters.len() as u32).to_le_bytes());
        for (id, c) in &self.counters {
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&c.reserved.to_le_bytes());
        }
        pager.update(&[self.pid], |p| {
            p[0][HEADER_LEN..HEADER_LEN + body.len()].copy_from_slice(&body);
        })?;
        pager.flush_log();
        Ok(())
    }
}

/// Allocate a page of counters starting each relation of `schema` whose single key is generated
/// by `sequence` past the largest key its key index `key(r)` holds, and skip a relation for
/// which `key` returns `None`. Returns its page id.
pub fn counter_page<'i, R: Runtime>(
    pager: &Pager<R>,
    schema: &Schema,
    key: impl Fn(RelId) -> Option<&'i Index>,
) -> Result<Pid, Error> {
    let pid = Sequences::create(pager)?;
    let mut counters = Sequences::open(pager, pid)?;
    for (i, rel) in schema.relations.iter().enumerate() {
        let r = RelId(u32::try_from(i).expect("a schema has fewer than 2^32 relations"));
        if rel.generate != Some(Generator::Sequence) || rel.key.len() != 1 {
            continue;
        }
        let Some(index) = key(r) else {
            continue;
        };
        let held = index
            .scan_prefix(pager, &[])?
            .into_iter()
            .filter_map(|e| match e.key.first() {
                Some(Value::I64(k)) => Some(*k),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        counters.advance(pager, r.0, held)?;
    }
    Ok(pid)
}

#[cfg(test)]
pub(crate) fn full_counter_page<R: Runtime>(pager: &Pager<R>, omitted: u32) -> Result<Pid, Error> {
    let pid = Sequences::create(pager)?;
    let mut sequences = Sequences::open(pager, pid)?;
    let room = (pager.page_size() - HEADER_LEN - 4) / 12;
    sequences.counters = (0..=u32::try_from(room).expect("the counter slot count fits u32"))
        .filter(|id| *id != omitted)
        .map(|id| {
            (
                id,
                Counter {
                    next: 1,
                    reserved: 1 + BLOCK,
                },
            )
        })
        .collect();
    assert_eq!(room, sequences.counters.len());
    sequences.persist(pager)?;
    Ok(pid)
}

#[cfg(test)]
#[path = "sequence_tests.rs"]
mod tests;
