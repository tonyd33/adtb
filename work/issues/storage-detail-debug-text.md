---
created-at: 2026-11-11
status: open
kind: bug
priority: 500
summary: "a Fatal Storage reply's detail is the Rust Debug text of mvcc::Error, PageError or the spill error, and the tests build the expected text with the same Debug formatting"
tags:
  - quality
  - engine
  - class-failure-flattened
---

# A Fatal Storage reply's detail is Rust Debug text

## Report

api's error mapping prints internal enums with `{:?}` into client-visible detail: crates/adtb/src/api/error.rs:369 `write::Error::Transactions(e) => Error::storage(format!("{e:?}"))`, error.rs:414 `exec::Error::Page(p) => Error::storage(format!("{p:?}"))`, and error.rs:418 the spill error as `{io:?}`. A client reads text like `Transactions(Corrupt(PageError { pid: Pid(3), damage: Unreadable }))`, and renaming a type or a derive in mvcc or storage changes it.

The tests take their expected value from the code under test: crates/adtb/src/api/failure_tests.rs:76 and :106-107 build it with `format!("{:?}", mvcc::Error::Corrupt(unreadable))` and `format!("{unreadable:?}")`, the tests in api/error.rs (about line 1061) with `Error::storage(format!("{damaged:?}"))`, and api/error_class_tests.rs the same, so no test fixes the text.

Direction: a `Display` for `mvcc::Error`, `PageError` and the spill error stating the page and the damage, printed by `of_mapped` and `of_exec`; expected details written as literal text.

Defect class: Damage or failure gives a panic, hang or flattened error.
