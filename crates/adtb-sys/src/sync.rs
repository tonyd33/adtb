//! Page memory's atomics and thread locals, Loom's models of them in tests under `cfg(adtb_loom)`.

#[cfg(all(test, adtb_loom))]
pub(crate) use loom::sync::atomic::{AtomicU64, Ordering, fence};
#[cfg(not(all(test, adtb_loom)))]
pub(crate) use std::{
    sync::atomic::{AtomicU64, Ordering, fence},
    thread_local,
};

#[cfg(all(test, adtb_loom))]
macro_rules! model_thread_local {
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const $init:block;) => {
        loom::thread_local!($(#[$attr])* $vis static $name: $t = $init;);
    };
}
#[cfg(all(test, adtb_loom))]
pub(crate) use model_thread_local as thread_local;
