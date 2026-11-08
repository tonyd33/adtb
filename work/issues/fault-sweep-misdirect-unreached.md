---
created-at: 2026-11-08
status: open
kind: bug
priority: 880
summary: "the write-fate sweep records a misdirected write as At, so it never proves a misdirect or the past-the-end landing was reached"
tags:
  - quality
  - sim
  - test
---

# The write-fate sweep passes without reaching a misdirected write

## Report

`every_write_meets_its_fate_from_draws_in_the_stated_order` (crates/adtb-sys/src/runtime/tests.rs:221) records each fate's kind through `write_kind` (tests.rs:191), which maps every `WriteFate::At(_)` to `"At"`, and asserts only that At, Fail, Full and Lost were reached (tests.rs:245). A misdirected write and the landing at `offset + len` when no other slot exists are both `At`, so the sweep never shows either branch ran; a draw sequence that never reaches a misdirect passes.

Fix direction: record At at its own offset, misdirected, and past the end as separate kinds, and assert all three are reached.
