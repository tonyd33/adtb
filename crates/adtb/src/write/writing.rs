//! Count the read-write transactions the calling thread holds open in process, and its runs of
//! the statements of transactions with an id.

use std::cell::Cell;
use std::marker::PhantomData;

use super::{Error, Rejection};

std::thread_local! {
    static WRITING: Cell<usize> = const { Cell::new(0) };
}

/// A read-write transaction the calling thread holds open, or its run of the statements of a
/// transaction with an id, until it drops.
#[derive(Debug)]
pub(crate) struct Writing(PhantomData<*const ()>);

impl Writing {
    pub(crate) fn enter() -> Writing {
        WRITING.with(|w| w.set(w.get() + 1));
        Writing(PhantomData)
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        WRITING.with(|w| w.set(w.get() - 1));
    }
}

/// Refuse a call that waits for every running writer when the calling thread holds one.
pub(crate) fn outside_writer() -> Result<(), Error> {
    if WRITING.with(Cell::get) == 0 {
        return Ok(());
    }
    Err(Error::Rejected(Rejection::Invalid {
        detail: "this thread holds an open read-write transaction, which a verify or a deferrable \
                 read would wait for"
            .to_owned(),
    }))
}
