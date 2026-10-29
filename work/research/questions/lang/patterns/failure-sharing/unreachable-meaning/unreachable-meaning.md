---
title: Unreachable marks a position a test, not exhaustiveness, excludes
pick: one
raised-by: [lang/patterns/failure-sharing/retest]
options:
  live:
    title: Restate Unreachable as a position the checker proves unreached
---
docs/design/kernel.md §3 gives `Unreachable` as "a position exhaustiveness
proves no evaluation reaches", and the reference interpreter reports
reaching it as such (crates/adtb-reference/src/eval.rs:817). The body's
re-destructuring puts it under constructors the condition excluded.

## lang/patterns/failure-sharing/unreachable-meaning/live

One sentence in the design file and the interpreter's message; reaching
it stays a broken invariant.
