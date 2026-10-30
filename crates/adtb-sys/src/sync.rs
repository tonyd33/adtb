//! The atomics and thread-local storage of page memory, Loom's models of them under `cfg(loom)`.

#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicU64, Ordering, fence};
#[cfg(not(loom))]
pub(crate) use std::{
    sync::atomic::{AtomicU64, Ordering, fence},
    thread_local,
};

#[cfg(loom)]
macro_rules! model_thread_local {
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const $init:block;) => {
        loom::thread_local!($(#[$attr])* $vis static $name: $t = $init;);
    };
}
#[cfg(loom)]
pub(crate) use model_thread_local as thread_local;
