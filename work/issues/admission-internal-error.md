---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 599
summary: "an internal error while compiling a constraint's delta reaches the user as a refusal detail, not an internal error"
closed: 2026-10-26
---

# Admission internal error

## Report

## Report

`crates/adtb/src/write/delta/mod.rs` turns `plan::Error::Internal` into a `Refusal` whose detail starts "a broken invariant". A refusal says the user's constraint is not admitted; a broken checker guarantee is not that. My ruling: admission returns the internal error itself, which fails the statement or migration as every other checker-guarantee error does.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as qpmmzlpu: `delta::compile` returns `Unadmitted::{Refused, Internal}`, and `Catalog::admits` passes a broken guarantee up as `catalog::Error::Internal`. It reaches users as `OpenError::Corrupt`; that is in operator-log-follow-ups.
