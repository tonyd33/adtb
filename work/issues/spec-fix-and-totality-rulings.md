---
created-at: 2026-10-25
status: open
kind: feature
priority: 190
summary: "seven rulings on totality, positivity, boundedness and key preservation from the normalisation work are not yet in the design space or the spec"
tags: [spec]
---

# Feature: spec rulings on fix, totality and boundedness

## Report

Found by the normalisation and analyses work
(`kernel-normalisation-and-analyses.md`). Each ruling below goes
through the design space as a question, then into the spec text.

1. `add_period(..., Reject)` is partial, since it can fail to produce a
   value. `docs/spec/typing.md` §10.1 calls it total.
2. Restriction `e ▷ S` is a positive position in the list of
   `docs/spec/semantics.md` §5.10.
3. Boundedness: a constructing operation counts only where its result
   can become an element of a relation of the stratum. A use only in a
   condition, a comparison or an aggregate key does not count. Bounded
   under this rule:
   - `fix a = (from u in users select key u) union (from x in a where { k: x } != { k: me } select x) in a`;
   - a backlink over an `Option` reference.
4. Projections: a primitive whose result is a component of an argument
   (`start`, `end`, `unwrap`, `value`) is not constructing.
5. `at` blocks are partial, because `HistoryUnavailable` is an abort.
6. Key-preserving (`docs/spec/queries.md` §10.1) also covers the binder
   of a generator over a `Set T` parameter or over a group's rows.
7. Partial constraint and check bodies are accepted. A fault while
   checking fails the statement. `docs/spec/constraints.md` states this
   explicitly.
