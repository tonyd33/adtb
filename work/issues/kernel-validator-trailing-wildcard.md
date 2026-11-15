---
created-at: 2026-11-15
status: open
kind: bug
priority: 550
summary: "the kernel validator's last match ends in _ => Ok(()), so a kernel kind added later passes validation unchecked"
tags:
  - quality
  - lang
  - class-copied-rules
---

# A kernel kind added later passes the validator unchecked

## Report

Found by the quality review of reviewed..582d7af5. The validator splits `Kind` over four matches, `node`, `rest`, `sets` and `writes` (crates/adtb-lang/src/kernel_valid.rs:308, 348, 446, 537), each passing the kinds it does not name to the next; `writes` ends in `_ => Ok(())` (line 602). Today every one of the 49 kinds is named in one of the four, so the arm is dead, but a kind added to `Kind` compiles here and validates as correct with no rule. `Build::computed` (kernel_build.rs:165) lists every kind and so fails to compile on a new one; the validator, the gate that release migration and registration rely on, does not.

Fix direction: replace the wildcard with the explicit list of kinds the earlier matches handled (as `computed` does), or restructure into one exhaustive match, so a new kind fails to compile until it has a rule.
