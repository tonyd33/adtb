//! Count the calling thread's runs of the statements of transactions with an id, in debug
//! builds and when `latch-checks` is on; count nothing otherwise.

#[cfg(any(debug_assertions, feature = "latch-checks"))]
mod counted {
    use std::cell::Cell;
    use std::marker::PhantomData;

    std::thread_local! {
        static WRITING: Cell<usize> = const { Cell::new(0) };
    }

    /// The calling thread's run of the statements of a transaction with an id, until it drops.
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

    /// Panic if the calling thread runs the statements of a transaction with an id.
    pub(crate) fn outside_writer() {
        assert!(
            WRITING.with(Cell::get) == 0,
            "this thread waits on writers while it runs a transaction with an id"
        );
    }
}

#[cfg(not(any(debug_assertions, feature = "latch-checks")))]
mod counted {
    pub(crate) struct Writing;

    impl Writing {
        pub(crate) fn enter() -> Writing {
            Writing
        }
    }

    pub(crate) fn outside_writer() {}
}

pub(crate) use counted::{Writing, outside_writer};
