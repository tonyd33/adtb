---
created-at: 2026-10-26
status: open
kind: bug
priority: 599
summary: "an internal error while compiling a constraint's delta reaches the user as a refusal detail, not an internal error"
---

# Admission internal error

## Report

## Report

`crates/adtb/src/write/delta/mod.rs` turns `plan::Error::Internal` into a `Refusal` whose detail starts "a broken invariant". A refusal says the user's constraint is not admitted; a broken checker guarantee is not that. My ruling: admission returns the internal error itself, which fails the statement or migration as every other checker-guarantee error does.
