//! Choose the host's real I/O backends for a test.

use std::io::Write;

use adtb_sys::os::{Backend, OsRuntime};

/// Returns `Backend::Uring { entries }` when the host allows it. When the host refuses io_uring,
/// returns `None` and says so on stderr if `ADTB_URING` is `absent`, and panics otherwise.
pub fn uring(entries: u32) -> Option<Backend> {
    let backend = Backend::Uring { entries };
    let Err(refused) = OsRuntime::try_new(".", backend) else {
        return Some(backend);
    };
    if std::env::var("ADTB_URING").as_deref() == Ok("absent") {
        let note = "ADTB_URING=absent: not run on io_uring";
        drop(writeln!(std::io::stderr().lock(), "{note}: {refused}"));
        return None;
    }
    panic!("{refused}; set ADTB_URING=absent on a host without io_uring");
}
