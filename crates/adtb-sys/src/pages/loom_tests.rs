use loom::sync::Arc;
use loom::thread;

use super::{PageMemory, PageState};
use crate::runtime::{ThreadId, Threads};

/// A thread interface that hands the model's scheduler the turn while a thread waits.
struct Model;

impl Threads for Model {
    fn spawn<F: FnOnce() + Send + 'static>(&self, _: &str, _: F) -> ThreadId {
        unreachable!()
    }
    fn current(&self) -> ThreadId {
        unreachable!()
    }
    fn park(&self) {
        unreachable!()
    }
    fn unpark(&self, _: ThreadId) {
        unreachable!()
    }
    fn yield_now(&self) {
        thread::yield_now();
    }
    fn join(&self, _: ThreadId) {
        unreachable!()
    }
}

const PID: u64 = 0;

/// Returns one resident page whose first two words hold `fill`, at version 1.
fn filled(fill: u8) -> Arc<PageMemory> {
    let m = PageMemory::reserve(4096, 1);
    m.lock_exclusive(PID, &Model).write(0, &[fill; 16]);
    Arc::new(m)
}

/// Returns word `w` of the page as a plain read through its bytes sees it.
fn plain(m: &PageMemory, w: usize) -> u64 {
    m.data.plain_read(PID as usize * (m.page_size / 8) + w)
}

/// Returns the first two words of the page as plain reads see them, as bytes.
fn plain_bytes(m: &PageMemory) -> [u8; 16] {
    let mut out = [0; 16];
    out[..8].copy_from_slice(&plain(m, 0).to_le_bytes());
    out[8..].copy_from_slice(&plain(m, 1).to_le_bytes());
    out
}

/// Returns the fill a validated copy at `version` must hold, given the fills of versions 1
/// and 2.
fn fill_at(version: u64, fills: [u8; 2]) -> u8 {
    match version {
        1 => fills[0],
        2 => fills[1],
        v => panic!("version {v} with one writer"),
    }
}

#[test]
fn a_shared_latch_sees_no_writer_or_all_of_one() {
    loom::model(|| {
        let m = filled(0);
        let w = Arc::clone(&m);
        let writer = thread::spawn(move || {
            let mut g = w.lock_exclusive(PID, &Model);
            g.write(0, &[1; 16]);
            g.write(0, &[2; 16]);
        });
        if let Some(s) = m.lock_shared(PID, &Model) {
            let (state, version) = m.state(PID);
            assert_eq!(PageState::Shared(1), state);
            assert_eq!(
                [fill_at(version, [0, 2]); 16],
                plain_bytes(&m),
                "a shared latch at version {version} saw another version's bytes"
            );
            drop(s);
        }
        writer.join().unwrap();
        assert_eq!((PageState::Unlocked, 2), m.state(PID));
    });
}

/// Latch the page exclusive, check it holds the last writer's fill, and fill it with `fill`.
fn overwrite(m: &PageMemory, fill: u8, other: u8) {
    let mut g = m.lock_exclusive(PID, &Model);
    let version = g.version;
    assert_eq!(
        [fill_at(version, [0, other]); 16],
        plain_bytes(m),
        "an exclusive latch at version {version} missed the last writer's stores"
    );
    g.write(0, &[fill; 16]);
}

#[test]
fn two_exclusive_latches_take_turns() {
    loom::model(|| {
        let m = filled(0);
        let w = Arc::clone(&m);
        let writer = thread::spawn(move || overwrite(&w, 1, 2));
        overwrite(&m, 2, 1);
        writer.join().unwrap();
        assert_eq!((PageState::Unlocked, 3), m.state(PID));
        let last = plain_bytes(&m);
        assert!(
            last == [1; 16] || last == [2; 16],
            "two writers left {last:?}"
        );
    });
}

#[test]
fn an_optimistic_read_sees_a_version_change_or_its_versions_page() {
    loom::model(|| {
        let m = filled(0);
        let w = Arc::clone(&m);
        let writer = thread::spawn(move || {
            w.lock_exclusive(PID, &Model).write(0, &[1; 16]);
        });
        if let Some(r) = m.optimistic(PID) {
            let mut out = [9; 16];
            r.read(0, &mut out);
            if r.validate() {
                assert_eq!(
                    [fill_at(r.version, [0, 1]); 16],
                    out,
                    "a validated read at version {} saw another version's bytes",
                    r.version
                );
            }
        }
        writer.join().unwrap();
    });
}

#[test]
fn an_optimistic_copy_sees_a_version_change_or_its_versions_page() {
    loom::model(|| {
        let m = filled(7);
        let w = Arc::clone(&m);
        let writer = thread::spawn(move || {
            w.lock_exclusive(PID, &Model).write(0, &[1; 16]);
        });
        if let Some(r) = m.optimistic(PID) {
            let out: [u8; 16] = r.copied(0, 16, |copy| copy.try_into().unwrap());
            if r.validate() {
                assert_eq!(
                    [fill_at(r.version, [7, 1]); 16],
                    out,
                    "a validated copy at version {} saw another version's bytes",
                    r.version
                );
            }
        }
        writer.join().unwrap();
    });
}

#[test]
fn a_second_optimistic_copy_sees_nothing_of_the_first() {
    loom::model(|| {
        let m = filled(0);
        let w = Arc::clone(&m);
        let writer = thread::spawn(move || {
            w.lock_exclusive(PID, &Model).write(0, &[1; 16]);
        });
        for _ in 0..2 {
            let Some(r) = m.optimistic(PID) else {
                continue;
            };
            let out: [u8; 16] = r.copied(0, 16, |copy| copy.try_into().unwrap());
            if r.validate() {
                assert_eq!(
                    [fill_at(r.version, [0, 1]); 16],
                    out,
                    "a validated copy at version {} saw another version's bytes",
                    r.version
                );
            }
        }
        writer.join().unwrap();
    });
}

#[test]
fn an_upgrade_finds_the_bytes_its_read_saw() {
    loom::model(|| {
        let m = filled(0);
        let w = Arc::clone(&m);
        let writer = thread::spawn(move || {
            w.lock_exclusive(PID, &Model).write(0, &[1; 16]);
        });
        if let Some(r) = m.optimistic(PID) {
            let mut out = [9; 16];
            r.read(0, &mut out);
            let version = r.version;
            if let Some(mut g) = r.upgrade() {
                assert_eq!(version, g.version);
                assert_eq!(
                    out,
                    plain_bytes(&m),
                    "an upgrade at version {version} found bytes its read did not see"
                );
                g.write(0, &[2; 16]);
            }
        }
        writer.join().unwrap();
    });
}

#[test]
fn an_upgrade_excludes_a_shared_latch() {
    loom::model(|| {
        let m = filled(0);
        let rd = Arc::clone(&m);
        let reader = thread::spawn(move || {
            if let Some(s) = rd.lock_shared(PID, &Model) {
                let version = rd.state(PID).1;
                assert_eq!(
                    [fill_at(version, [0, 2]); 16],
                    plain_bytes(&rd),
                    "a shared latch at version {version} saw another version's bytes"
                );
                drop(s);
            }
        });
        let r = m.optimistic(PID).expect("no writer latches the page");
        if let Some(mut g) = r.upgrade() {
            g.write(0, &[2; 16]);
        }
        reader.join().unwrap();
    });
}

#[test]
fn the_clock_never_refuses_an_upgrade() {
    loom::model(|| {
        let m = filled(0);
        let c = Arc::clone(&m);
        let clock = thread::spawn(move || {
            c.mark(PID);
        });
        let r = m.optimistic(PID).expect("no writer latches the page");
        assert!(r.upgrade().is_some(), "the clock's mark refused an upgrade");
        clock.join().unwrap();
    });
}

#[test]
fn share_hands_readers_the_page_it_wrote() {
    loom::model(|| {
        let m = filled(0);
        let rd = Arc::clone(&m);
        let reader = thread::spawn(move || {
            if let Some(r) = rd.optimistic(PID) {
                let mut out = [9; 16];
                r.read(0, &mut out);
                if r.validate() {
                    assert_eq!(
                        [fill_at(r.version, [0, 1]); 16],
                        out,
                        "a validated read at version {} saw another version's bytes",
                        r.version
                    );
                }
            }
        });
        let mut g = m.lock_exclusive(PID, &Model);
        g.write(0, &[1; 16]);
        let s = g.share();
        assert_eq!([1; 16], plain_bytes(&m));
        drop(s);
        reader.join().unwrap();
        assert_eq!((PageState::Unlocked, 2), m.state(PID));
    });
}

#[test]
fn the_clock_never_evicts_a_latched_page() {
    loom::model(|| {
        let m = filled(7);
        assert!(!m.mark(PID));
        let c = Arc::clone(&m);
        let clock = thread::spawn(move || sweep(&c));
        if let Some(s) = m.lock_shared(PID, &Model) {
            assert_eq!(
                [7; 16],
                plain_bytes(&m),
                "the clock evicted a page under a shared latch"
            );
            drop(s);
        }
        clock.join().unwrap();
        let (state, version) = m.state(PID);
        assert!(
            matches!(
                (state, version),
                (PageState::Unlocked | PageState::Marked, 1) | (PageState::Evicted, 2)
            ),
            "the clock and a shared latch left {state:?} at version {version}"
        );
    });
}

/// Evict the page if the clock passed over it once already and no one latches it.
fn sweep(m: &PageMemory) {
    if m.mark(PID)
        && let Some(g) = m.try_lock_exclusive(PID)
    {
        g.evict();
    }
}

#[test]
fn a_page_filled_after_eviction_keeps_its_fill() {
    loom::model(|| {
        let m = filled(7);
        assert!(!m.mark(PID));
        let c = Arc::clone(&m);
        let clock = thread::spawn(move || sweep(&c));
        let mut g = m.lock_exclusive(PID, &Model);
        if !g.resident() {
            g.write(0, &[8; 16]);
        }
        drop(g);
        clock.join().unwrap();
        let (state, version) = m.state(PID);
        let bytes = plain_bytes(&m);
        assert!(
            bytes == [7; 16] || bytes == [8; 16] || state == PageState::Evicted,
            "a fill after eviction was lost: {bytes:?} in {state:?} at version {version}"
        );
    });
}

#[test]
fn an_optimistic_read_clearing_the_mark_races_the_clock() {
    loom::model(|| {
        let m = filled(7);
        assert!(!m.mark(PID));
        let c = Arc::clone(&m);
        let clock = thread::spawn(move || sweep(&c));
        if let Some(r) = m.optimistic(PID) {
            let mut out = [9; 16];
            r.read(0, &mut out);
            if r.validate() {
                assert_eq!(1, r.version);
                assert_eq!(
                    [7; 16], out,
                    "a validated read saw the clock evict the page"
                );
            }
        }
        clock.join().unwrap();
        let (state, version) = m.state(PID);
        assert!(
            matches!(
                (state, version),
                (PageState::Unlocked | PageState::Marked, 1) | (PageState::Evicted, 2)
            ),
            "the clock and an optimistic read left {state:?} at version {version}"
        );
    });
}

#[test]
fn the_clock_never_marks_over_an_exclusive_latch() {
    loom::model(|| {
        let m = filled(0);
        let c = Arc::clone(&m);
        let clock = thread::spawn(move || {
            c.mark(PID);
        });
        m.lock_exclusive(PID, &Model).write(0, &[3; 16]);
        clock.join().unwrap();
        let (state, version) = m.state(PID);
        assert!(
            matches!(state, PageState::Unlocked | PageState::Marked) && version == 2,
            "the clock and an exclusive latch left {state:?} at version {version}"
        );
        assert_eq!([3; 16], plain_bytes(&m));
    });
}
