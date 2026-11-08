//! The background scrub: one thread per engine that reads every page of the file over each
//! `ScrubInterval`, without a call touching them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::runtime::{Clock, Threads};
use crate::storage::{Options, PageKind, Pager};
use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::profile;
use adtb_testkit::scratch::Scratch;

#[test]
fn a_page_checked_against_a_writer_on_real_threads_finds_nothing() {
    let dir = Scratch::new("scrub", "hammer");
    let rt = OsRuntime::new(dir.path(), Backend::Pool { threads: 4 });
    let mut o = Options::new("h.adt");
    o.pool_frames = 64;
    o.page_size = 65536;
    let pager = Arc::new(Pager::create(rt.clone(), &o).expect("the pager"));
    let page = pager.allocate(1, PageKind::HeapSlotted, 7).expect("a page");
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (pager, stop) = (pager.clone(), stop.clone());
        rt.spawn("writer", move || {
            let mut i = 0u8;
            while !stop.load(Ordering::Relaxed) {
                i = i.wrapping_add(1);
                pager
                    .update(&[page], |p| p[0][1024..].fill(i))
                    .expect("an update");
                pager.flush_pages(1);
            }
        })
    };
    let limit = Duration::from_millis(profile::deep(1500));
    let deadline = rt.now() + limit;
    let checkers: Vec<_> = (0..4)
        .map(|c| {
            let (pager, rt2) = (pager.clone(), rt.clone());
            rt.spawn("checker", move || {
                let mut checks = 0u64;
                while rt2.now() < deadline {
                    let log = pager.read_log(true);
                    for _ in 0..200 {
                        let found = pager.check_page(page, &log);
                        assert_eq!(
                            Vec::<crate::storage::Finding>::new(),
                            found,
                            "checker {c}, check {checks}"
                        );
                        checks += 1;
                    }
                }
            })
        })
        .collect();
    for c in checkers {
        rt.join(c);
    }
    stop.store(true, Ordering::Relaxed);
    rt.join(writer);
}
