---
created-at: 2026-10-27
status: open
kind: bug
priority: 897
summary: "the process_options write test accepts any error, and the host-fault test's message omits the result"
tags:
  - quality
  - test
---

# The process_options write test accepts any error, and the host-fault message omits the result

## Report

- `crates/adtb-sim/tests/embedding.rs:579` asserts only `refused.is_err()` for a write into `sys::process_options`, so a parse error or an internal error passes. Assert the specific reason.
- `crates/adtb-sim/tests/host_fns.rs:295` writes `assert!(faulted(got), "{what}")`; `got` moves into the helper, so a failure does not show what came back. Put `got` in the message.
