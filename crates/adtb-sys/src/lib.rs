//! Every `unsafe` block of the engine, and the runtime interfaces: I/O, clock, threads, entropy.

#[cfg(feature = "test-heap-cap")]
extern crate adtb_heap_cap as _;

pub mod buf;
pub mod crc32c;
pub mod lock;
#[cfg(target_os = "linux")]
pub mod os;
pub mod pages;
pub mod runtime;
mod sync;
