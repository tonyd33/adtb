//! The sequences the search runs: 4 to 12 operations from a seed, steered to free pages and
//! allocate them again, and to write every kind of page that holds data or evidence.

use std::collections::BTreeSet;

use super::world::{self, Op, Rel};

/// A seeded generator: a sequence is a function of its seed alone.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// The rows of each relation a sequence under construction holds, and the next id of each.
#[derive(Default)]
struct Shape {
    users: BTreeSet<i64>,
    docs: BTreeSet<i64>,
    tags: BTreeSet<i64>,
    next: i64,
    version: u8,
    recycled: bool,
    damaged: bool,
}

impl Shape {
    fn rows(&mut self, rel: Rel) -> &mut BTreeSet<i64> {
        match rel {
            Rel::Users => &mut self.users,
            Rel::Docs => &mut self.docs,
            Rel::Tags => &mut self.tags,
            Rel::Scraps => unreachable!("the model holds no rows of scraps"),
        }
    }

    /// Returns the next migration, after a delete of every row of `tags` when it drops `tags`.
    fn migrate(&mut self) -> Vec<Op> {
        self.version += 1;
        let mut out = Vec::new();
        if self.version == world::LAST && !self.tags.is_empty() {
            out.push(Op::Delete(
                Rel::Tags,
                std::mem::take(&mut self.tags).into_iter().collect(),
            ));
        }
        out.push(Op::Migrate);
        out
    }

    fn insert(&mut self, rel: Rel, n: i64) -> Op {
        let ids: Vec<i64> = (self.next..self.next + n).collect();
        self.next += n;
        self.rows(rel).extend(ids.iter().copied());
        Op::Insert(rel, ids)
    }

    /// Returns a write of `rel` of every row, most rows or a few, or `None` when `rel` holds no
    /// row.
    fn change(&mut self, rel: Rel, d: &mut Draw, delete: bool) -> Option<Op> {
        let rows: Vec<i64> = self.rows(rel).iter().copied().collect();
        if rows.is_empty() {
            return None;
        }
        let share = [100, 85, 30][d.below(3) as usize];
        let ids: Vec<i64> = rows.into_iter().filter(|_| d.chance(share)).collect();
        if ids.is_empty() {
            return None;
        }
        if delete {
            for id in &ids {
                self.rows(rel).remove(id);
            }
            Some(Op::Delete(rel, ids))
        } else {
            Some(Op::Update(rel, ids))
        }
    }
}

/// The first seed whose sequence is steered: damage, a reopen that marks it, then a recycle of
/// the log.
pub const STEERED: u64 = 1001;

/// Returns the sequence of seed `seed`. One seed in three adds `tags` and drops it. A seed from
/// [`STEERED`] on damages a page early, reopens, checkpoints, then recycles the log.
pub fn sequence(seed: u64) -> Vec<Op> {
    if seed < STEERED {
        return drawn(seed);
    }
    let mut ops: Vec<Op> = drawn(seed)
        .into_iter()
        .filter(|o| !matches!(o, Op::Damage | Op::Recycle))
        .collect();
    let at = ops.len().min(2);
    ops.splice(
        at..at,
        [
            Op::Damage,
            Op::Checkpoint,
            Op::Reopen,
            Op::Checkpoint,
            Op::Recycle,
        ],
    );
    ops.truncate(12);
    ops
}

/// Returns the sequence the draws of seed `seed` make.
fn drawn(seed: u64) -> Vec<Op> {
    let mut d = Draw(seed);
    let mut s = Shape::default();
    let mut ops = vec![s.insert(Rel::Users, 10 + d.below(50) as i64)];
    if d.chance(70) {
        ops.push(s.insert(Rel::Docs, 1 + d.below(4) as i64));
    }
    let len = 4 + d.below(9) as usize;
    let dropping = seed.is_multiple_of(3);
    while ops.len() < len {
        if dropping && s.version < world::LAST && d.chance(35) {
            ops.extend(s.migrate());
            continue;
        }
        let rel = if s.version == 1 && d.chance(25) {
            Rel::Tags
        } else if d.chance(65) {
            Rel::Users
        } else {
            Rel::Docs
        };
        let op = match d.below(100) {
            15..=24 => s.change(rel, &mut d, false),
            25..=39 => {
                let deleted = s.change(rel, &mut d, true);
                if let Some(op) = deleted {
                    ops.push(op);
                }
                ops.push(Op::Vacuum(rel));
                Some(s.insert(rel, 1 + d.below(30) as i64))
            }
            40..=47 => Some(Op::Vacuum(rel)),
            48..=57 => Some(Op::Checkpoint),
            58..=64 if !s.recycled => {
                s.recycled = true;
                Some(Op::Recycle)
            }
            65..=71 if s.version < world::LAST => {
                ops.extend(s.migrate());
                None
            }
            72..=79 => Some(Op::Reopen),
            80..=89 => Some(Op::Crash),
            90..=95 if !s.damaged => {
                s.damaged = true;
                ops.push(Op::Damage);
                Some(Op::Checkpoint)
            }
            _ => Some(s.insert(rel, 1 + d.below(30) as i64)),
        };
        ops.extend(op);
    }
    if !s.damaged && d.chance(50) {
        let at = 1 + d.below(ops.len() as u64 - 1) as usize;
        ops.splice(at..at, [Op::Damage, Op::Checkpoint]);
    }
    ops.truncate(12);
    ops
}
