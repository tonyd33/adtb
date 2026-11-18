//! The code generated from the fixture shop's log and bundle, and the runner of the manual's Rust
//! examples.

#[cfg(test)]
extern crate adtb_lang as _;

pub mod manual;

pub mod shop {
    include!(concat!(env!("OUT_DIR"), "/shop.rs"));
}
