use std::collections::BTreeMap;

use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process::finished;

use crate::mvcc::{Begun, Hold, Scope, Transactions};
use crate::storage::Pager;
use crate::test_world::options;

use super::deployment::Fact;
use super::kept::{KeptField, KeptRow};
use super::{Catalog, Error, Writer};

/// One transaction's write of keyed storage.
#[derive(Clone, Debug)]
enum Put {
    Fact(Fact),
    /// Replace the kept rows of this group of relation 339 by one row holding this number.
    Kept(&'static str, i64),
}

/// What keyed storage holds: each family's owner, each bundle's latched renewal and the kept
/// rows of relation 339 by group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Held {
    families: BTreeMap<String, i64>,
    expired: BTreeMap<i64, i64>,
    kept: BTreeMap<String, Vec<i64>>,
}

impl Held {
    /// Apply `put` as a transaction that committed after every write held.
    fn apply(&mut self, put: &Put) {
        match put {
            Put::Fact(Fact::Family(family, role)) => {
                self.families.insert(family.clone(), *role);
            }
            Put::Fact(Fact::Expired(bundle, renewed)) => {
                let latest = self.expired.entry(*bundle).or_insert(*renewed);
                *latest = (*latest).max(*renewed);
            }
            Put::Fact(other) => panic!("no case writes {other:?}"),
            Put::Kept(group, n) => {
                self.kept.insert((*group).to_owned(), vec![*n]);
            }
        }
    }
}

fn kept_row(group: &str, n: i64) -> KeptRow {
    KeptRow {
        relation: 339,
        group: group.to_owned(),
        ctor: None,
        fields: vec![KeptField::Int(n)],
    }
}

/// Returns what `catalog` holds at the latest commit.
fn read(catalog: &Catalog, pager: &Pager<SimRuntime>, txns: &Transactions<SimRuntime>) -> Held {
    let deployed = catalog.deployed(pager, txns).expect("the facts read");
    let mut kept: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for r in catalog.kept(pager, txns).expect("the kept rows read") {
        let [KeptField::Int(n)] = r.fields[..] else {
            panic!("a kept row of the case holds one number: {r:?}");
        };
        kept.entry(r.group).or_default().push(n);
    }
    Held {
        families: deployed.families,
        expired: deployed.expired,
        kept,
    }
}

/// One step of a transaction: its begin, its write and its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Begin,
    Write,
    End,
}

const STEPS: [Step; 3] = [Step::Begin, Step::Write, Step::End];

/// Returns every interleaving of the steps of two transactions, each as the transaction (0 or
/// 1) of each step in order.
fn interleavings() -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    for mask in 0u32..64 {
        if mask.count_ones() == 3 {
            out.push((0..6).map(|i| usize::from(mask & (1 << i) != 0)).collect());
        }
    }
    out
}

/// A transaction of a case as it runs.
struct Running {
    begun: Option<Begun>,
    hold: Option<Hold>,
    failed: Option<Error>,
}

/// Returns what the store holds after the steps of `order` run `puts`, and which transactions
/// committed.
fn run(seed: u64, puts: &[Put; 2], order: &[usize]) -> (Held, [bool; 2]) {
    let mut out = None;
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt: SimRuntime| {
        let pager = Pager::create(rt.clone(), &options(64)).expect("create");
        let (catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let mut running: Vec<Running> = (0..2)
            .map(|_| Running {
                begun: None,
                hold: None,
                failed: None,
            })
            .collect();
        let mut next = [0usize; 2];
        let mut committed = [false; 2];
        for &t in order {
            let step = STEPS[next[t]];
            next[t] += 1;
            let r = &mut running[t];
            match step {
                Step::Begin => {
                    r.begun = Some(txns.begin(&pager).expect("a transaction"));
                    r.hold = Some(txns.take(Scope::Every).expect("the hold"));
                }
                Step::Write => {
                    let hold = r.hold.as_ref().expect("begun");
                    let x = r.begun.as_ref().expect("begun").xid();
                    let w = Writer::new(hold, x);
                    let written = match &puts[t] {
                        Put::Fact(f) => {
                            catalog
                                .deployment()
                                .insert(&pager, &txns, w, std::slice::from_ref(f))
                        }
                        Put::Kept(group, n) => catalog.replace_kept(
                            &pager,
                            &txns,
                            w,
                            (339, group),
                            &[kept_row(group, *n)],
                        ),
                    };
                    r.failed = written.err();
                }
                Step::End => {
                    let begun = r.begun.take().expect("begun");
                    if r.failed.is_none() {
                        txns.commit(&pager, begun).expect("the commit").done();
                        committed[t] = true;
                    } else {
                        txns.abort(&pager, begun).expect("the abort");
                    }
                    drop(r.hold.take());
                }
            }
        }
        for r in &running {
            if let Some(e) = &r.failed {
                assert!(
                    matches!(e, Error::Conflict(_)),
                    "a write fails only by conflict: {e:?}"
                );
            }
        }
        out = Some((read(&catalog, &pager, &txns), committed));
    });
    out.expect("the case ran")
}

/// Returns the transactions of `order` that commit when each write of `puts` conflicts with a
/// write of the same key by a transaction its snapshot does not see, and what they leave.
fn expected(puts: &[Put; 2], same_key: bool, order: &[usize]) -> (Held, [bool; 2]) {
    let at = |t: usize, step: usize| {
        order
            .iter()
            .enumerate()
            .filter(|(_, o)| **o == t)
            .nth(step)
            .map(|(i, _)| i)
            .expect("each transaction takes three steps")
    };
    let overlap = at(1, 0) < at(0, 2) && at(0, 0) < at(1, 2);
    let mut committed = [true; 2];
    if same_key && overlap {
        let later = if at(0, 1) < at(1, 1) { 1 } else { 0 };
        committed[later] = false;
    }
    let mut ends: Vec<usize> = (0..2).filter(|t| committed[*t]).collect();
    ends.sort_by_key(|t| at(*t, 2));
    let mut held = Held::default();
    for t in ends {
        held.apply(&puts[t]);
    }
    (held, committed)
}

#[test]
fn two_writers_of_one_key_of_keyed_storage_conflict_and_of_two_keys_both_commit() {
    let family = |f: &str, role| Put::Fact(Fact::Family(f.to_owned(), role));
    let expired = |bundle, at| Put::Fact(Fact::Expired(bundle, at));
    let cases: [([Put; 2], bool); 7] = [
        ([family("app", 1), family("app", 2)], true),
        ([family("app", 1), family("web", 2)], false),
        ([expired(7, 50), expired(7, 30)], true),
        ([expired(7, 30), expired(7, 50)], true),
        ([expired(7, 30), expired(8, 50)], false),
        ([Put::Kept("g", 1), Put::Kept("g", 2)], true),
        ([Put::Kept("a", 1), Put::Kept("b", 2)], true),
    ];
    let orders = interleavings();
    assert_eq!(20, orders.len());
    let mut conflicted = 0;
    for (puts, same_key) in &cases {
        for (i, order) in orders.iter().enumerate() {
            let want = expected(puts, *same_key, order);
            let got = run(i as u64, puts, order);
            assert_eq!(want, got, "{puts:?} in order {order:?}");
            conflicted += usize::from(!got.1[0] || !got.1[1]);
        }
    }
    assert_eq!(
        5 * 18,
        conflicted,
        "every overlapping pair of one key conflicts"
    );
}
