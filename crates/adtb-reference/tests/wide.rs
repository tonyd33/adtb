//! A block of tens of thousands of bindings, evaluated on a 2 MB thread.

use std::fmt::Write;
use std::thread;

use adtb_testkit::fixture::*;
use adtb_testkit::writes::fixture;

const STACK: usize = 2 << 20;

#[expect(
    clippy::disallowed_methods,
    reason = "goes if the test thread's stack may follow `RUST_MIN_STACK`"
)]
fn on_small_stack(body: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .stack_size(STACK)
        .spawn(body)
        .expect("the test thread starts")
        .join()
        .unwrap_or_else(|p| std::panic::resume_unwind(p));
}

#[test]
fn a_block_of_tens_of_thousands_of_bindings_returns_its_first() {
    on_small_stack(|| {
        let mut f = fixture();
        let mut block = String::from("transact\n let a0 = 0\n");
        for i in 1..60_000 {
            writeln!(block, " let a{i} = {i}").expect("a string takes text");
        }
        block.push_str(" return a0\n");
        let out = f.run_with(&block, &[]).expect("the block runs");
        assert_eq!(int(0), out.result);
    });
}
