---
created-at: 2026-11-18
status: open
kind: bug
priority: 700
summary: "cargo conformance --include PATTERN that names no case reports 0 passed, every case skipped, and exits 0"
tags:
  - quality
  - tooling
  - test
---

# cargo conformance passes when --include selects no case

## Report

`verdict` (`crates/adtb-conformance/src/runner/mod.rs:346-351`) marks each case whose name does not hold the `--include` text `Skipped`, and `Report::ok` (mod.rs:274) counts only failures and selection problems. A pattern that matches no case runs nothing and succeeds.

```
cargo conformance --include no_such_case
```

Actual: `0 passed, 0 failed, 0 updated, 744 skipped`, exit 0.

Expected: a failure naming the pattern, as `select` already reports `no cases under PATH` for an empty directory (mod.rs:162-167). A typo in `--include` then reads as a pass in a script or an agent's report.

Fix: after the run, when `include` is set and no case was run (every verdict `Skipped`), add a problem `--include PATTERN selects no case`; test it in tests/cli.rs. The receipt of the snapshot runner records the sibling case, `--reference --file-name` under the scripts corpus selecting nothing with exit 0, as a known limit; the same check would cover it if the lead wants it.
