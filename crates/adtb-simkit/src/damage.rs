//! Change the bytes of a database's data file between runs, at the offsets its layout gives a page.

use std::collections::BTreeMap;
use std::ops::Range;

use adtb::storage::{HEADER_LEN, Options, Pid};

/// Every file of a simulated device, by path.
pub type Files = BTreeMap<String, Vec<u8>>;

/// A data file: its path, and the page size its header records.
#[derive(Clone, Copy, Debug)]
pub struct DataFile<'a> {
    name: &'a str,
    page_size: usize,
}

impl<'a> DataFile<'a> {
    pub const fn new(name: &'a str, page_size: usize) -> DataFile<'a> {
        DataFile { name, page_size }
    }

    /// Returns the data file a pager opened with `options` keeps.
    pub fn of(options: &'a Options) -> DataFile<'a> {
        DataFile::new(&options.name, options.page_size as usize)
    }

    /// Returns the byte range page `page` takes in the file.
    pub fn range(&self, page: Pid) -> Range<usize> {
        let start = usize::try_from(page.0).expect("a page id fits an offset") * self.page_size;
        start..start + self.page_size
    }

    /// Returns the bytes of the data file in `files`.
    pub fn bytes<'f>(&self, files: &'f Files) -> &'f [u8] {
        files
            .get(self.name)
            .unwrap_or_else(|| panic!("the device holds no {}", self.name))
    }

    fn bytes_mut<'f>(&self, files: &'f mut Files) -> &'f mut Vec<u8> {
        files
            .get_mut(self.name)
            .unwrap_or_else(|| panic!("the device holds no {}", self.name))
    }

    /// Returns page `page` of the data file in `files`.
    pub fn page<'f>(&self, files: &'f Files, page: Pid) -> &'f [u8] {
        &self.bytes(files)[self.range(page)]
    }

    /// Returns page `page` of the data file in `files`, to change in place.
    pub fn page_mut<'f>(&self, files: &'f mut Files, page: Pid) -> &'f mut [u8] {
        let range = self.range(page);
        &mut self.bytes_mut(files)[range]
    }

    /// Invert byte `at` of page `page`'s body, the bytes past its header.
    ///
    /// Preconditions:
    /// - `at` lies inside the body: below the page size less the header.
    pub fn flip(&self, files: &mut Files, page: Pid, at: usize) {
        assert!(
            at < self.page_size - HEADER_LEN,
            "byte {at} of a body of {} bytes",
            self.page_size - HEADER_LEN
        );
        self.page_mut(files, page)[HEADER_LEN + at] ^= 0xff;
    }

    /// Write zeros over every byte of page `page`.
    pub fn zero(&self, files: &mut Files, page: Pid) {
        self.page_mut(files, page).fill(0);
    }

    /// Write `image` over page `page`.
    pub fn replace(&self, files: &mut Files, page: Pid, image: &[u8]) {
        self.page_mut(files, page).copy_from_slice(image);
    }

    /// Cut the file to its first `pages` pages.
    pub fn truncate(&self, files: &mut Files, pages: usize) {
        self.bytes_mut(files).truncate(pages * self.page_size);
    }

    /// Lengthen the file with zeros to hold its first `pages` pages.
    ///
    /// Preconditions:
    /// - The file holds at most `pages` pages.
    pub fn extend(&self, files: &mut Files, pages: usize) {
        let data = self.bytes_mut(files);
        assert!(
            data.len() <= pages * self.page_size,
            "a file of {} bytes is longer than {pages} pages",
            data.len()
        );
        data.resize(pages * self.page_size, 0);
    }

    /// Cut the file to its first `len` bytes.
    pub fn cut(&self, files: &mut Files, len: usize) {
        self.bytes_mut(files).truncate(len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{PAGE, finished, options};
    use adtb::storage::{PageKind, Pager};
    use adtb_sim::{Config, Sim};

    fn file(pages: usize) -> Files {
        let bytes = (0..pages * PAGE).map(|i| (i / PAGE) as u8).collect();
        Files::from([("db".to_owned(), bytes)])
    }

    /// Returns the files of a pager holding one page of each kind, and those pages.
    fn every_kind() -> (Files, Vec<(PageKind, Pid)>) {
        let mut sim = Sim::new(1, Config::default());
        let mut pages = Vec::new();
        finished(&mut sim, |rt| {
            let pager = Pager::create(rt, &options(8)).expect("create");
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
        let options = options(8);
        let data = DataFile::of(&options);
        let (before, pages) = every_kind();
        assert_eq!(13, pages.len());
        for (kind, pid) in pages {
            for at in [0, PAGE / 2, PAGE - HEADER_LEN - 1] {
                let mut after = before.clone();
                data.flip(&mut after, pid, at);
                let old = data.bytes(&before);
                let new = data.bytes(&after);
                let changed: Vec<usize> = (0..old.len()).filter(|&i| old[i] != new[i]).collect();
                let start = data.range(pid).start;
                assert_eq!(vec![start + HEADER_LEN + at], changed, "{kind:?} byte {at}");
            }
        }
    }

    #[test]
    #[should_panic(expected = "byte 4072 of a body of 4072 bytes")]
    fn a_flip_past_the_body_is_refused() {
        DataFile::new("db", PAGE).flip(&mut file(4), Pid(2), PAGE - HEADER_LEN);
    }

    #[test]
    fn a_page_is_the_page_sized_run_at_its_id() {
        let files = file(4);
        let data = DataFile::new("db", PAGE);
        assert_eq!(&[3u8; PAGE][..], data.page(&files, Pid(3)));
        let mut zeroed = files.clone();
        data.zero(&mut zeroed, Pid(1));
        assert_eq!(&[0u8; PAGE][..], data.page(&zeroed, Pid(1)));
        assert_eq!(data.page(&files, Pid(2)), data.page(&zeroed, Pid(2)));
        let mut cut = files;
        data.truncate(&mut cut, 2);
        assert_eq!(2 * PAGE, data.bytes(&cut).len());
    }
}
