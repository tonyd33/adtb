---
created-at: 2026-11-16
status: open
kind: bug
priority: 650
summary: "structure-check counts a disallowed_methods expect as prod in a file that a #[cfg(test)] mod in another file includes, so crash_tests/mod.rs sits on a prod row"
tags:
  - tooling
  - quality
  - class-none
---

# structure-check counts an expect as prod in a file only a cfg(test) mod of another file includes

## Report

Found by the quality review of reviewed..main (c93c399e..92dbbc64) on main 92dbbc64.

crates/adtb-structure/src/main.rs:6-8 states an expect counts as test when an item, statement or expression on its path carries `#[cfg(test)]`. The parse is per file, so it cannot see the attribute of a `mod` declared in another file. crates/adtb-bench/src/native/crash_tests/mod.rs is compiled only through crates/adtb-bench/src/native/driver/mod.rs:278-280 (`#[cfg(test)] #[path = "../crash_tests/mod.rs"] mod crash_tests;`), and its expect at line 14 stands on a prod row: work/tools/structure-allowlist:161 `disallowed_methods prod 1 crates/adtb-bench/src/native/crash_tests/mod.rs`. The path patterns in work/tools/structure-check do not match `crash_tests/` either. The resolved structure-check-test-modules ticket recorded pages/mod.rs as the one site.

Fix direction: resolve `mod` declarations from each crate root and carry a parent's cfg(test) into the child file, or class a file reached only through a cfg(test) `mod` as test. A structure-check-test case with a test module in its own file.
