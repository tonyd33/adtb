---
created-at: 2026-10-29
status: open
kind: bug
priority: 865
summary: "enforcing reports a stored migration that no longer replays as Refused from changes but as Unaccepted from the schema step, against its doc"
tags:
  - quality
  - engine
---

# enforcing reports a migration that no longer replays as two different errors

## Report

`enforcing` (`crates/adtb/src/catalog/readers.rs:286-350`) documents failing only with `Error::Corrupt` or with `Error::Unaccepted` when a log migration no longer applies keeping the drops. The `changes(&l.before, &before, &file, &l.text)?` call (about line 331) returns `Error::Refused` from `at.steps(...)` (readers.rs about 259) and `steps::built` (`crates/adtb/src/catalog/steps.rs:22`) unmapped, while the `l.before.schema()` call just above maps its failure to `Unaccepted(l.number, ...)`. `enforcing` runs at catalog open (`Readers::reopened`, `crates/adtb/src/catalog/pending.rs:281`) and on release and settle (`unhold`, pending.rs:351), so at open such a log reads as a refused migration instead of `Unaccepted`.

Latent: it needs a log entry that once applied and now fails `steps`.

Fix: map the errors from `changes` inside `enforcing` to `Unaccepted(l.number, ...)`.
