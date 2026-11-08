use adtb_harness::damage::{DataFile, Files};
use adtb_harness::process::finished;
use adtb_sim::{Config, Sim};

use super::{HEADER_LEN, Options, PageKind, Pager, Pid};

const PAGE: usize = 4096;

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 8,
        ..Options::new("app.adt")
    }
}

/// Returns the files of a pager holding one page of each kind, and those pages.
fn every_kind() -> (Files, Vec<(PageKind, Pid)>) {
    let mut sim = Sim::new(1, Config::default());
    let mut pages = Vec::new();
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options()).expect("create");
        pages = (1..)
            .map_while(PageKind::from_u16)
            .map(|kind| (kind, pager.allocate(1, kind, 7).expect("allocate")))
            .collect();
        pager.flush_log();
        pager.flush_pages(usize::MAX);
    });
    (sim.files(), pages)
}

#[test]
fn a_flip_changes_one_byte_of_its_page_past_the_header_of_every_kind() {
    let options = options();
    let data = DataFile::new(&options.name, PAGE);
    let (before, pages) = every_kind();
    assert_eq!(13, pages.len());
    for (kind, pid) in pages {
        for at in [0, PAGE / 2, PAGE - HEADER_LEN - 1] {
            let mut after = before.clone();
            data.flip(&mut after, pid, at);
            let old = data.bytes(&before);
            let new = data.bytes(&after);
            let changed: Vec<usize> = (0..old.len()).filter(|&i| old[i] != new[i]).collect();
            let start = usize::try_from(pid.0).expect("a page id fits an offset") * PAGE;
            assert_eq!(vec![start + HEADER_LEN + at], changed, "{kind:?} byte {at}");
        }
    }
}
