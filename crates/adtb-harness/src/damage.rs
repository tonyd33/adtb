//! Change the bytes of a database's data file between runs, at the offsets its layout gives a page.

use std::collections::BTreeMap;
use std::ops::Range;

/// The length of the header that starts every page of the data file.
pub const HEADER_LEN: usize = 24;

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

    /// Returns the byte range page `page` takes in the file.
    pub fn range(&self, page: impl Into<u64>) -> Range<usize> {
        let start =
            usize::try_from(page.into()).expect("a page number fits an offset") * self.page_size;
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
    pub fn page<'f>(&self, files: &'f Files, page: impl Into<u64>) -> &'f [u8] {
        &self.bytes(files)[self.range(page)]
    }

    /// Returns page `page` of the data file in `files`, to change in place.
    pub fn page_mut<'f>(&self, files: &'f mut Files, page: impl Into<u64>) -> &'f mut [u8] {
        let range = self.range(page);
        &mut self.bytes_mut(files)[range]
    }

    /// Invert byte `at` of page `page`'s body, the bytes past its header.
    ///
    /// Preconditions:
    /// - `at` lies inside the body: below the page size less the header.
    pub fn flip(&self, files: &mut Files, page: impl Into<u64>, at: usize) {
        assert!(
            at < self.page_size - HEADER_LEN,
            "byte {at} of a body of {} bytes",
            self.page_size - HEADER_LEN
        );
        self.page_mut(files, page)[HEADER_LEN + at] ^= 0xff;
    }

    /// Write zeros over every byte of page `page`.
    pub fn zero(&self, files: &mut Files, page: impl Into<u64>) {
        self.page_mut(files, page).fill(0);
    }

    /// Write `image` over page `page`.
    pub fn replace(&self, files: &mut Files, page: impl Into<u64>, image: &[u8]) {
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

    const PAGE: usize = 4096;

    fn file(pages: usize) -> Files {
        let bytes = (0..pages * PAGE).map(|i| (i / PAGE) as u8).collect();
        Files::from([("db".to_owned(), bytes)])
    }

    #[test]
    #[should_panic(expected = "byte 4072 of a body of 4072 bytes")]
    fn a_flip_past_the_body_is_refused() {
        DataFile::new("db", PAGE).flip(&mut file(4), 2u64, PAGE - HEADER_LEN);
    }

    #[test]
    fn a_page_is_the_page_sized_run_at_its_id() {
        let files = file(4);
        let data = DataFile::new("db", PAGE);
        assert_eq!(&[3u8; PAGE][..], data.page(&files, 3u64));
        let mut zeroed = files.clone();
        data.zero(&mut zeroed, 1u64);
        assert_eq!(&[0u8; PAGE][..], data.page(&zeroed, 1u64));
        assert_eq!(data.page(&files, 2u64), data.page(&zeroed, 2u64));
        let mut cut = files;
        data.truncate(&mut cut, 2);
        assert_eq!(2 * PAGE, data.bytes(&cut).len());
    }
}
