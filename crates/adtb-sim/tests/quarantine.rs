use std::collections::BTreeSet;
use std::time::Duration;

use adtb::runtime::{Clock, Io, Op, Runtime, Threads};
use adtb::storage::{Options, Pager, Quarantine};
use adtb_sim::{Config, Outcome, Sim};

const NAME: &str = "app.adt";
const PAGE: usize = 4096;

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        segment_size: 64 * 1024,
        pool_frames: 16,
        max_pages: 1024,
        ..Options::new(NAME)
    }
}

fn create(sim: &mut Sim) {
    assert_eq!(
        Outcome::Finished,
        sim.run(|rt| {
            Pager::create(rt, &options()).unwrap();
        })
    );
}

fn with_pager<T>(sim: &mut Sim, body: impl FnOnce(&Pager<adtb_sim::SimRuntime>) -> T) -> T {
    let mut out = None;
    assert_eq!(
        Outcome::Finished,
        sim.run(|rt| {
            let (p, _) = Pager::open(rt, &options()).unwrap();
            out = Some(body(&p));
        })
    );
    out.unwrap()
}

#[test]
fn a_new_database_quarantines_nothing() {
    let mut sim = Sim::new(1, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| {
        assert!(p.quarantined().is_empty());
        assert!(!p.is_quarantined(Quarantine::Heap(1)));
    });
}

#[test]
fn marks_survive_a_restart_until_released() {
    let mut sim = Sim::new(2, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| {
        p.quarantine(Quarantine::Heap(7)).unwrap();
        p.quarantine(Quarantine::Index(3)).unwrap();
        p.quarantine(Quarantine::Heap(7)).unwrap();
        assert!(p.is_quarantined(Quarantine::Heap(7)));
        assert!(
            !p.is_quarantined(Quarantine::Heap(3)),
            "heap and index ids are apart"
        );
    });
    with_pager(&mut sim, |p| {
        assert_eq!(
            BTreeSet::from([Quarantine::Heap(7), Quarantine::Index(3)]),
            p.quarantined()
        );
        p.release(Quarantine::Heap(7)).unwrap();
    });
    with_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([Quarantine::Index(3)]), p.quarantined());
        assert!(p.verify().findings.is_empty());
    });
}

#[test]
fn a_mark_is_durable_once_quarantine_returns() {
    for seed in 0..8 {
        let mut sim = Sim::new(seed, Config::default());
        create(&mut sim);
        sim.crash_at(sim.now() + Duration::from_secs(1));
        let outcome = sim.run(|rt| {
            let (p, _) = Pager::open(rt.clone(), &options()).unwrap();
            p.quarantine(Quarantine::Heap(40 + seed as u32)).unwrap();
            let mut io = rt.io();
            io.submit(Op::Timer {
                at: rt.now() + Duration::from_secs(10),
            });
            loop {
                rt.park();
            }
        });
        assert_eq!(Outcome::Crashed, outcome);
        with_pager(&mut sim, |p| {
            assert!(
                p.is_quarantined(Quarantine::Heap(40 + seed as u32)),
                "seed {seed}"
            );
        });
    }
}

#[test]
fn a_database_mark_covers_every_object() {
    let mut sim = Sim::new(3, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| p.quarantine(Quarantine::Database).unwrap());
    with_pager(&mut sim, |p| {
        assert!(p.is_quarantined(Quarantine::Heap(9)));
        assert!(p.is_quarantined(Quarantine::Index(9)));
    });
}

#[test]
fn a_damaged_mark_page_quarantines_the_database() {
    let mut sim = Sim::new(4, Config::default());
    create(&mut sim);
    let page = with_pager(&mut sim, |p| {
        p.quarantine(Quarantine::Index(5)).unwrap();
        p.quarantine_page()
    });
    with_pager(&mut sim, |_| ());
    let mut files = sim.files();
    files.get_mut(NAME).unwrap()[page.0 as usize * PAGE + 200] ^= 0x10;
    let mut sim = Sim::with_files(4, Config::default(), files);
    with_pager(&mut sim, |p| {
        assert!(p.is_quarantined(Quarantine::Database));
        assert!(p.quarantine(Quarantine::Heap(1)).is_err());
    });
}
