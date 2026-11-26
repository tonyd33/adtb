use std::collections::BTreeMap;

use adtb_lang::schema::{Generator, Schema};
use adtb_lang::types::RelId;

use crate::btree::{self, Index};
use crate::key::Value;
use crate::runtime::Runtime;
use crate::storage::{self, HEADER_LEN, PageError, PageKind, Pager, Pid, Shape};

const FORMAT: u16 = 0x5351;

/// Why a counter could not be drawn, advanced or stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CounterError {
    Page(PageError),
    /// The key index a new page's counter starts past fails to read, other than on a page.
    Index(btree::Error),
    /// The counter has drawn every value of its integer range.
    Exhausted,
    /// The page holds this many counters, and a new one does not fit.
    Full(usize),
}

/// Bytes a counter takes on its page: its relation id and its reservation.
const COUNTER_LEN: usize = 12;

/// Returns how many counters a page of `page_size` bytes holds: past its header, a 4-byte count
/// and [`COUNTER_LEN`] bytes for each.
fn room(page_size: usize) -> usize {
    (page_size - HEADER_LEN - 4) / COUNTER_LEN
}

/// Returns the refusal of a counter past the `room` its page holds.
pub fn past_room(room: usize) -> String {
    format!(
        "more than {room} generated counters: each takes {COUNTER_LEN} bytes of the one page that holds them"
    )
}

impl From<PageError> for CounterError {
    fn from(e: PageError) -> CounterError {
        CounterError::Page(e)
    }
}

impl From<btree::Error> for CounterError {
    fn from(e: btree::Error) -> CounterError {
        match e {
            btree::Error::Page(p) => CounterError::Page(p),
            e => CounterError::Index(e),
        }
    }
}

/// Returns how `page` is out of shape as a page of counters.
fn counters_fault(page: &[u8]) -> Option<Shape> {
    if storage::kind(page) != PageKind::Catalog as u16 {
        Some(storage::kind_fault(page))
    } else if storage::format(page) != FORMAT {
        Some(Shape::Format)
    } else {
        None
    }
}

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
    /// Refuse `pid`, from then on, if it is not a page of counters.
    pub fn open<R: Runtime>(pager: &Pager<R>, pid: Pid) -> Result<Sequences, PageError> {
        let page = loop {
            let page = pager.read(pid)?;
            if counters_fault(&page).is_none() {
                break page;
            }
            drop(page);
            if let Some(e) = pager.refuse_if(pid, |p| counters_fault(p)) {
                return Err(e);
            }
        };
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

    #[cfg(any(test, feature = "testing"))]
    pub fn root(&self) -> Pid {
        self.pid
    }

    /// Returns the next value of counter `id`, at least `floor`.
    pub fn draw<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        id: u32,
        floor: i64,
    ) -> Result<i64, CounterError> {
        let mut c = self.counter(id);
        let v = c.next.max(floor);
        c.next = v.checked_add(1).ok_or(CounterError::Exhausted)?;
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
    ) -> Result<(), CounterError> {
        let mut c = self.counter(id);
        c.next = c.next.max(held.saturating_add(1));
        c.reserved = c.reserved.max(c.next);
        self.replace(pager, id, c)
    }

    /// Returns the value counter `id` draws next with no floor.
    pub fn peek(&self, id: u32) -> i64 {
        self.counter(id).next
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
    ) -> Result<(), CounterError> {
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
    fn persist<R: Runtime>(&self, pager: &Pager<R>) -> Result<(), CounterError> {
        let room = room(pager.page_size());
        if self.counters.len() > room {
            return Err(CounterError::Full(room));
        }
        let mut body = Vec::with_capacity(4 + COUNTER_LEN * self.counters.len());
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
) -> Result<Pid, CounterError> {
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
pub(crate) fn full_counter_page<R: Runtime>(
    pager: &Pager<R>,
    omitted: u32,
) -> Result<Pid, CounterError> {
    let pid = Sequences::create(pager)?;
    let mut sequences = Sequences::open(pager, pid)?;
    let room = room(pager.page_size());
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
