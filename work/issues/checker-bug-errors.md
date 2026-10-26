---
created-at: 2026-10-25
status: open
kind: feature
priority: 594
summary: "the fatal-split ruling says every broken invariant crashes, and the spec gives a client no error for a checker bug"
tags: [quality, spec]
---

# Feature: state checker-bug errors in the rulings and the spec

## Report

`internal-error-path` made a broken checker guarantee fail the statement. `project/arch/errors/fatal-split` and `docs/design/README.md` section 4 still say every broken invariant crashes the process, and no reason in `docs/spec/catalog.md` section 4 covers it.

My ruling, to record as a refinement of fatal-split: an invariant the checker guarantees about a submitted program (the types and normal forms that reach planning, evaluation and write evaluation) fails the statement with an internal error when broken, and the process keeps running; the transaction stays open. Invariants over engine state (pages, log, latches, heap and index layout, the stored catalog) still crash and recover from the log. A client receives the failure as an `Internal { what: Text }` constructor of the statement error sum.
