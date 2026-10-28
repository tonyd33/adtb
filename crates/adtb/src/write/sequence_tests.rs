use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use adtb_sys::os::{Backend, Faults, OsRuntime};

use crate::storage::{Damage, HEADER_LEN, Options, PageKind, Pager};
use crate::write::Error;

use super::{BLOCK, Counter, Sequences};

struct Fixture {
    path: PathBuf,
    rt: OsRuntime,
    pager: Pager<OsRuntime>,
    seqs: Sequences,
}

impl Fixture {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let parent =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/sequence-regressions");
        std::fs::create_dir_all(&parent).expect("create fixture parent");
        let path = loop {
            let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("{name}-{}-{nonce}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create fixture directory: {error}"),
            }
        };
        let rt = OsRuntime::with_faults(&path, Backend::Pool { threads: 1 }, Faults::default());
        let pager = Pager::create(rt.clone(), &Self::options()).expect("create pager");
        let pid = Sequences::create(&pager).expect("create sequences");
        let seqs = Sequences::open(&pager, pid).expect("open sequences");
        Self {
            path,
            rt,
            pager,
            seqs,
        }
    }

    fn options() -> Options {
        Options {
            pool_frames: 16,
            max_pages: 1024,
            ..Options::new("sequence.adt")
        }
    }

    fn full(name: &str) -> Self {
        let mut f = Self::new(name);
        let room = (f.pager.page_size() - HEADER_LEN - 4) / 12;
        f.seqs.counters = (0..u32::try_from(room).expect("counter slots fit u32"))
            .map(|id| {
                (
                    id,
                    Counter {
                        next: 1,
                        reserved: 1 + BLOCK,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        f.seqs.persist(&f.pager).expect("fill durable counter page");
        f
    }

    fn crash(self) -> Self {
        let Self {
            path,
            rt,
            pager,
            seqs,
        } = self;
        let root = seqs.root();
        rt.crash();
        drop(pager);
        drop(rt);
        let rt = OsRuntime::with_faults(&path, Backend::Pool { threads: 1 }, Faults::default());
        let (pager, recovery) = Pager::open(rt.clone(), &Self::options()).expect("recover pager");
        assert!(
            recovery.applied > 0,
            "recovery replays the forced reservation log"
        );
        assert!(recovery.damaged.is_empty(), "{recovery:?}");
        let seqs = Sequences::open(&pager, root).expect("open recovered sequences");
        Self {
            path,
            rt,
            pager,
            seqs,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        self.pager
            .read(self.seqs.root())
            .expect("counter page")
            .to_vec()
    }
}

fn refused<T: Debug>(result: Result<T, Error>, detail: &str) {
    assert!(
        matches!(&result, Err(Error::Unsupported(got)) if got == detail),
        "expected {detail:?}, got {result:?}"
    );
}

#[test]
fn a_refused_draw_cannot_return_an_unreserved_identity_on_retry() {
    let mut f = Fixture::full("draw-capacity");
    let extra = u32::try_from(f.seqs.counters.len()).unwrap();
    let before = f.seqs.counters.clone();
    let bytes = f.bytes();
    let detail = format!("more than {extra} generated counters");
    for _ in 0..3 {
        refused(f.seqs.draw(&f.pager, extra, 1), &detail);
        assert_eq!(before, f.seqs.counters);
        assert_eq!(bytes, f.bytes());
    }
    assert_eq!(1, f.seqs.draw(&f.pager, 0, 1).unwrap());
    f = f.crash();
    assert!(!f.seqs.counters.contains_key(&extra));
    refused(f.seqs.draw(&f.pager, extra, 1), &detail);
    assert!(f.seqs.draw(&f.pager, 0, 1).unwrap() > 1);
    f.pager.close();
}

#[test]
fn a_refused_advance_cannot_report_an_unpersisted_reservation_on_retry() {
    let mut f = Fixture::full("advance-capacity");
    let extra = u32::try_from(f.seqs.counters.len()).unwrap();
    let before = f.seqs.counters.clone();
    let bytes = f.bytes();
    let detail = format!("more than {extra} generated counters");
    for _ in 0..3 {
        refused(f.seqs.advance(&f.pager, extra, 7), &detail);
        assert_eq!(before, f.seqs.counters);
        assert_eq!(bytes, f.bytes());
    }
    f.seqs.advance(&f.pager, 0, 100).unwrap();
    f = f.crash();
    assert!(!f.seqs.counters.contains_key(&extra));
    refused(f.seqs.advance(&f.pager, extra, 7), &detail);
    assert!(f.seqs.draw(&f.pager, 0, 1).unwrap() > 100);
    f.pager.close();
}

#[test]
fn drawing_at_the_integer_limit_refuses_without_overflow() {
    let mut f = Fixture::full("draw-overflow");
    let before = f.seqs.counters.clone();
    let bytes = f.bytes();
    for id in [0, u32::MAX] {
        assert_eq!(
            Err(Error::SequenceExhausted),
            f.seqs.draw(&f.pager, id, i64::MAX)
        );
        assert_eq!(before, f.seqs.counters);
        assert_eq!(bytes, f.bytes());
    }
    assert_eq!(1, f.seqs.draw(&f.pager, 0, 1).unwrap());
    f.pager.close();
}

#[test]
fn exhaustion_survives_recovery_and_accepts_explicit_maximum_keys() {
    let mut f = Fixture::new("exhaustion");
    assert_eq!(
        i64::MAX - 1,
        f.seqs.draw(&f.pager, 0, i64::MAX - 1).unwrap()
    );
    f.seqs.advance(&f.pager, 1, i64::MAX).unwrap();
    f = f.crash();
    for id in [0, 1] {
        assert_eq!(Err(Error::SequenceExhausted), f.seqs.draw(&f.pager, id, 1));
        f.seqs.advance(&f.pager, id, i64::MAX).unwrap();
        assert_eq!(Err(Error::SequenceExhausted), f.seqs.draw(&f.pager, id, 1));
    }
    f.pager.close();
}

#[test]
fn drawing_within_a_reservation_does_not_force_another_log_record() {
    let mut f = Fixture::new("reservation");
    assert_eq!(1, f.seqs.draw(&f.pager, 0, 1).unwrap());
    let durable = f.pager.durable();
    let bytes = f.bytes();
    for value in 2..=BLOCK {
        assert_eq!(value, f.seqs.draw(&f.pager, 0, 1).unwrap());
        assert_eq!(durable, f.pager.durable());
        assert_eq!(bytes, f.bytes());
    }
    assert_eq!(BLOCK + 1, f.seqs.draw(&f.pager, 0, 1).unwrap());
    assert!(f.pager.durable() > durable);
    for _ in 0..2 {
        let reserved = f.seqs.counter(0).reserved;
        f = f.crash();
        assert!(f.seqs.draw(&f.pager, 0, 1).unwrap() >= reserved);
    }
    f.pager.close();
}

#[test]
fn damaged_counter_page_refuses_retries_without_advancing_cached_reservations() {
    let mut f = Fixture::new("page-error");
    assert_eq!(1, f.seqs.draw(&f.pager, 0, 1).unwrap());
    let before = f.seqs.counters.clone();
    for _ in 0..32 {
        f.pager.allocate(1, PageKind::Catalog, 0).unwrap();
    }
    f.rt.set_faults(Faults {
        latent_p: 1.0,
        ..Faults::default()
    });
    let Err(page) = f.pager.read(f.seqs.root()) else {
        panic!("the evicted counter page must suffer the injected read fault");
    };
    assert_eq!(Damage::Unreadable, page.damage);
    f.rt.set_faults(Faults::default());
    for id in [0, 1] {
        for _ in 0..2 {
            let draw = f.seqs.draw(&f.pager, id, 100);
            assert!(matches!(draw, Err(Error::Page(p)) if p == page), "{draw:?}");
            assert_eq!(before, f.seqs.counters);
            let advance = f.seqs.advance(&f.pager, id, 100);
            assert!(
                matches!(advance, Err(Error::Page(p)) if p == page),
                "{advance:?}"
            );
            assert_eq!(before, f.seqs.counters);
        }
    }
    f.pager.close();
}

#[test]
fn lower_floors_and_explicit_keys_never_rewind_a_counter() {
    let mut f = Fixture::new("lower-bounds");
    f.seqs.advance(&f.pager, 0, -10).unwrap();
    assert_eq!(1, f.seqs.draw(&f.pager, 0, -10).unwrap());
    let durable = f.pager.durable();
    f.seqs.advance(&f.pager, 0, -100).unwrap();
    assert_eq!(2, f.seqs.draw(&f.pager, 0, -100).unwrap());
    assert_eq!(durable, f.pager.durable());
    f.seqs.advance(&f.pager, 0, 100).unwrap();
    f.seqs.advance(&f.pager, 0, 7).unwrap();
    assert_eq!(101, f.seqs.draw(&f.pager, 0, 1).unwrap());
    f = f.crash();
    assert!(f.seqs.draw(&f.pager, 0, 1).unwrap() > 101);
    f.pager.close();
}
