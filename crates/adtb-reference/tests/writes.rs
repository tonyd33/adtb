//! Writes, delete actions, upserts, blocks and the statement boundary, driven through the front
//! end. The examples of the writes part of the spec whose outcome it states are here.

#[path = "writes/mod.rs"]
mod cases;

#[test]
fn runtime_write_cases() {
    let (families, steps) = cases::run();
    eprintln!("runtime writes: {families} families, {steps} steps executed");
}
