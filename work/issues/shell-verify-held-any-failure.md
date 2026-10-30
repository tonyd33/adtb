---
created-at: 2026-10-30
status: open
kind: bug
priority: 800
summary: "the test that adtb verify refuses a held file asserts only exit 2, which any failed open gives"
tags:
  - quality
  - test
---

# The test that verify refuses a held file accepts any failure to open

## Report

`crates/adtb-shell/tests/shell.rs:600` `verify_exits_2_while_another_process_holds_the_file` asserts only exit status 2, which every failed open gives. If the holder had not taken the lock and the run failed for another reason, or the file were refused as corrupt, the test passes. Nothing pins that the lock is what stopped it.

The sibling `verify_of_a_missing_file_exits_2_and_says_why` checks stderr.

Fix direction: assert stderr names the file being in use, the text of `OpenError::Locked`.
