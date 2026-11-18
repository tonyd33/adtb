use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process::finished;

use crate::mvcc::Xid;
use crate::storage::{PageKind, Pager, Pid};
use crate::test_world::options;

use super::{Changing, FORMAT, Root, Unbegun, Unended, begin, end, end_found};

#[test]
fn the_root_page_round_trips() {
    for (changing, stored) in [
        (Changing::None, 0),
        (Changing::Schema(Xid(1025)), 1025),
        (Changing::NamingSequences, u64::MAX),
        (Changing::Reindexing, u64::MAX - 1),
    ] {
        let root = Root {
            commit_log: Pid(7),
            large: (1, Pid(3)),
            roots: (2, Pid(9)),
            next_heap: 70,
            changing,
            sequences: Pid(77),
            history_large: (71, Pid(78)),
            record: (72, Pid(79)),
        };
        let mut page = vec![0; 4096];
        root.write(&mut page);
        assert_eq!(root, Root::read(&page));
        assert!(page[..24].iter().all(|&b| b == 0));
        let at = 24 + 8 + 4 + 4 + 8 + 8 + 8;
        assert_eq!(stored.to_le_bytes(), page[at..at + 8], "{changing:?}");
    }
}

/// Returns one change in progress of each kind, the schema change of transaction `x`.
fn every_kind(x: Xid) -> [Changing; 3] {
    let kinds = [
        Changing::Schema(x),
        Changing::NamingSequences,
        Changing::Reindexing,
    ];
    for (i, k) in kinds.into_iter().enumerate() {
        let at = match k {
            Changing::None => None,
            Changing::Schema(_) => Some(0),
            Changing::NamingSequences => Some(1),
            Changing::Reindexing => Some(2),
        };
        assert_eq!(Some(i), at, "{k:?}");
    }
    kinds
}

/// Run `body` over a fresh catalog root page.
fn with_root(seed: u64, body: impl FnOnce(&Pager<SimRuntime>, Pid)) {
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(16)).expect("create");
        let root = pager
            .allocate(FORMAT, PageKind::Catalog, 0)
            .expect("a root page");
        pager
            .update(&[root], |p| Root::created(Pid(0), 0).write(p[0]))
            .expect("the root page");
        body(&pager, root);
    });
}

fn changing(pager: &Pager<SimRuntime>, root: Pid) -> Changing {
    Root::read(&pager.read(root).expect("the root page")).changing
}

#[test]
fn a_change_begun_over_another_in_progress_is_refused_with_it() {
    let kept = every_kind(Xid(1025));
    let begun = every_kind(Xid(1026));
    let mut got = Vec::new();
    with_root(36, |pager, root| {
        for k in kept {
            let held = begin(pager, root, k).expect("a change over none begins");
            for b in begun {
                let refused = begin(pager, root, b).map(drop);
                got.push((k, b, refused, changing(pager, root)));
            }
            end(pager, held, |_| {}).expect("the change ends");
            got.push((k, k, Ok(()), changing(pager, root)));
        }
    });
    let mut want = Vec::new();
    for k in kept {
        for b in begun {
            want.push((k, b, Err(Unbegun::Kept(k)), k));
        }
        want.push((k, k, Ok(()), Changing::None));
    }
    assert_eq!(want, got);
}

#[test]
fn an_end_over_another_change_in_progress_is_refused_with_it() {
    let begun = every_kind(Xid(1025));
    let mut found = vec![Changing::None];
    found.extend(every_kind(Xid(1026)));
    let mut got = Vec::new();
    with_root(37, |pager, root| {
        for b in begun {
            for &f in found.iter().filter(|&&f| f != b) {
                let held = begin(pager, root, b).expect("a change over none begins");
                end_found(pager, root).expect("the change is cleared");
                let other = (f != Changing::None)
                    .then(|| begin(pager, root, f).expect("the other change begins"));
                let ended = end(pager, held, |r| r.sequences = Pid(9)).map(drop);
                let after = Root::read(&pager.read(root).expect("the root page"));
                got.push((b, f, ended, after.changing, after.sequences));
                if let Some(other) = other {
                    end(pager, other, |_| {}).expect("the other change ends");
                }
            }
        }
    });
    let mut want = Vec::new();
    for b in begun {
        for &f in found.iter().filter(|&&f| f != b) {
            want.push((b, f, Err(Unended::Moved(f)), f, Pid(0)));
        }
    }
    assert_eq!(want, got);
}
