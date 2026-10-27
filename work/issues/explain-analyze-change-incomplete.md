---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 693
summary: explain analyze leaves temp relation effects out of the change and counts one actual row for a write
closed: 2026-10-26
tags:
  - quality
  - engine
---

# explain analyze of a write to a temp relation reports an empty change and one actual row

## Report

`run_here` (`crates/adtb/src/api/command.rs:199-225`) takes the temp relation effects out before `boundary`, so `Outcome.inserted` and `Outcome.deleted` never include them, and `explain analyze` reports `change: {}` for an insert into a temp relation. Separately, `rows` (`crates/adtb/src/write/eval.rs:240-246`) counts 1 for any result that is not a set or list, so every write shows `actual rows 1`.

```adt
temp relation t : { a: I64 } key a
explain analyze insert t { a: 1 }
```

Actual: `change: {}` and `actual rows 1`.

Expected: the change names one row inserted into `t`, and a write's result has no row count (or 0).

Fix: carry temp relation effects in the outcome, or count them in the analysed change, and leave `rows` out for a change or unit result.

## 2026-10-26: done

A statement's `Outcome` carries its temp relation effects in `inserted` and `deleted`, normalised with the rest, so `explain analyze insert t { a: 1 }` reports `t.t: -0 +1`. A statement of type `Change` counts 0 actual rows. Tests in `adtb-sim/tests/commands.rs`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the ebug track.
