//! The Rust examples of the manual's pages, each run as a test.

#![expect(
    dead_code,
    reason = "each example declares the generated code and uses part of it"
)]

mod rust_api {
    include!(concat!(
        env!("OUT_DIR"),
        "/reference/rust-api.md/src/examples.rs"
    ));
}

mod errors {
    include!(concat!(
        env!("OUT_DIR"),
        "/reference/errors.md/src/examples.rs"
    ));
}
