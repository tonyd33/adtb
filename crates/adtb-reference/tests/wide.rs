//! Terms with thousands of arms, items or alternatives, evaluated on a 2 MB thread.

use std::fmt::Write;
use std::thread;

use adtb_testkit::fixture::*;
use adtb_testkit::writes::fixture;

const STACK: usize = 2 << 20;

const WIDTH: i64 = 4000;

#[expect(
    clippy::disallowed_methods,
    reason = "only a spawned thread fixes the stack size a test body runs on"
)]
fn on_small_stack(body: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .stack_size(STACK)
        .spawn(body)
        .expect("the test thread starts")
        .join()
        .unwrap_or_else(|p| std::panic::resume_unwind(p));
}

fn users(f: &Fixture) -> usize {
    f.table("users").as_set().expect("a set").len()
}

fn scalar_match(arm: impl Fn(i64) -> String) -> String {
    let mut s = String::from("match n");
    for i in 0..WIDTH {
        write!(s, " | {i} -> {}", arm(i)).expect("a string takes text");
    }
    s
}

#[test]
fn a_scalar_match_of_thousands_of_arms_takes_its_last_arm() {
    on_small_stack(|| {
        let f = fixture();
        let text = scalar_match(|i| (i * 10).to_string()) + " | _ -> -1";
        let n = [("n", "I64", int(WIDTH - 1))];
        assert_eq!(Ok(int((WIDTH - 1) * 10)), f.eval_with(&text, &n));
        let n = [("n", "I64", int(WIDTH))];
        assert_eq!(Ok(int(-1)), f.eval_with(&text, &n));
    });
}

#[test]
fn an_or_pattern_of_thousands_of_alternatives_takes_its_last_alternative() {
    on_small_stack(|| {
        let f = fixture();
        let alts: Vec<String> = (0..WIDTH).map(|i| i.to_string()).collect();
        let text = format!("match n | {} -> 1 | _ -> 0", alts.join(" | "));
        let n = [("n", "I64", int(WIDTH - 1))];
        assert_eq!(Ok(int(1)), f.eval_with(&text, &n));
    });
}

#[test]
fn a_write_match_of_thousands_of_arms_takes_its_last_arm() {
    on_small_stack(|| {
        let mut f = fixture();
        let write =
            scalar_match(|i| format!(r#"insert users {{ name: "u{i}" }}"#)) + " | _ -> skip";
        f.run_with(&write, &[("n", "I64", int(WIDTH - 1))])
            .expect("the last arm inserts");
        assert_eq!(
            set([text(&format!("u{}", WIDTH - 1))]),
            f.value(&format!(
                r#"from u in users where u.name == "u{}" select u.name"#,
                WIDTH - 1
            ))
        );
    });
}

#[test]
fn a_block_of_thousands_of_items_runs_each() {
    on_small_stack(|| {
        let mut f = fixture();
        let mut block = String::from("transact\n let a0 = 0\n");
        for i in 1..WIDTH {
            writeln!(block, " let a{i} = a{} + 1", i - 1).expect("a string takes text");
            writeln!(block, r#" insert users {{ name: "b{i}" }}"#).expect("a string takes text");
        }
        writeln!(block, " return a{}", WIDTH - 1).expect("a string takes text");
        let before = users(&f);
        let out = f.run_with(&block, &[]).expect("the block runs");
        assert_eq!(int(WIDTH - 1), out.result);
        assert_eq!(before + (WIDTH - 1) as usize, users(&f));
    });
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
