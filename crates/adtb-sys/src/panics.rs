//! End the process at a panic raised in engine code, before anything unwinds.
//!
//! A thread runs engine code while it holds an [`Inside`] mark and no [`Outside`] mark taken
//! after it. A panic raised there ends the process from the panic hook, after the hook it
//! replaced has run: the thread's latches, transactions and holds are still held when the process
//! ends. A [`FatalIo`] payload and a panic raised anywhere else unwind as they would without the
//! hook. A payload resumed with `resume_unwind` passes no hook.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::panic::{self, PanicHookInfo};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::runtime::FatalIo;

/// What a panic in engine code does in place of aborting the process. It runs in the panic hook
/// on the panicking thread, and the panic unwinds once it returns.
pub type End = Box<dyn Fn(&PanicHookInfo<'_>)>;

thread_local! {
    static INSIDE: Cell<usize> = const { Cell::new(0) };
    static END: RefCell<Option<End>> = const { RefCell::new(None) };
}

static QUIET_FATAL_IO: AtomicBool = AtomicBool::new(false);

/// A mark that the calling thread runs engine code, held until it drops.
#[derive(Debug)]
#[must_use]
pub struct Inside(PhantomData<*const ()>);

/// Mark the calling thread as running engine code until the mark drops, installing the hook
/// first.
pub fn inside() -> Inside {
    install();
    INSIDE.with(|n| n.set(n.get() + 1));
    Inside(PhantomData)
}

impl Drop for Inside {
    fn drop(&mut self) {
        INSIDE.with(|n| n.set(n.get() - 1));
    }
}

/// A mark that the calling thread runs the embedder's code within engine code, held until it
/// drops.
#[derive(Debug)]
#[must_use]
pub struct Outside {
    inside: usize,
    _thread: PhantomData<*const ()>,
}

/// Mark the calling thread as running the embedder's code until the mark drops: a panic there
/// unwinds.
pub fn outside() -> Outside {
    Outside {
        inside: INSIDE.with(|n| n.replace(0)),
        _thread: PhantomData,
    }
}

impl Drop for Outside {
    fn drop(&mut self) {
        INSIDE.with(|n| n.set(self.inside));
    }
}

/// A replacement of what a panic in engine code on the calling thread does, held until it drops.
#[must_use]
pub struct Ending {
    prior: Option<End>,
    _thread: PhantomData<*const ()>,
}

/// Make a panic in engine code on the calling thread run `end` in place of aborting the process,
/// until the returned mark drops. The panic unwinds once `end` returns.
///
/// Preconditions: the caller is a simulator, and it ends the simulated process before any other
/// thread takes what the unwind releases.
pub fn end_with(end: End) -> Ending {
    install();
    Ending {
        prior: END.with(|e| e.replace(Some(end))),
        _thread: PhantomData,
    }
}

impl std::fmt::Debug for Ending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ending").finish_non_exhaustive()
    }
}

impl Drop for Ending {
    fn drop(&mut self) {
        let prior = self.prior.take();
        END.with(|e| *e.borrow_mut() = prior);
    }
}

/// Keep the hook's reports of a [`FatalIo`] payload off stderr for the rest of the process. The
/// direct diagnostic [`crate::runtime::fatal`] writes is not suppressed.
pub fn quiet_fatal_io() {
    install();
    QUIET_FATAL_IO.store(true, Ordering::Relaxed);
}

/// Install the panic hook, once in the process, over the hook in place. A call while the
/// calling thread panics installs nothing.
pub fn install() {
    static HOOK: Once = Once::new();
    if std::thread::panicking() {
        return;
    }
    HOOK.call_once(|| {
        let prior = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let fatal_io = info.payload().is::<FatalIo>();
            if !(QUIET_FATAL_IO.load(Ordering::Relaxed) && fatal_io) {
                prior(info);
            }
            if !fatal_io && INSIDE.try_with(Cell::get).unwrap_or(0) > 0 {
                end(info);
            }
        }));
    });
}

fn end(info: &PanicHookInfo<'_>) {
    let replaced = END
        .try_with(|e| match &*e.borrow() {
            Some(end) => {
                end(info);
                true
            }
            None => false,
        })
        .unwrap_or(false);
    if !replaced {
        std::process::abort();
    }
}

/// Returns a payload standing for the one the panic of `info` carries: a copy of a
/// `&'static str` or a `String`, else the `String` of `info`'s report.
pub fn payload_of(info: &PanicHookInfo<'_>) -> Box<dyn Any + Send> {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        Box::new(*text)
    } else if let Some(text) = payload.downcast_ref::<String>() {
        Box::new(text.clone())
    } else {
        Box::new(info.to_string())
    }
}
