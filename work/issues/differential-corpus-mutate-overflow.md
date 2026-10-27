---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 699
summary: differential corpus_mutants_equal_reference overflows under the deep profile with attempt to add with overflow in mutate
closed: 2026-10-26
tags:
  - test
  - sim
---

# Bug: corpus mutate overflows an integer token under the deep profile

## Report

`adtb-testkit::differential corpus_mutants_equal_reference` overflows under the deep profile: `attempt to add with overflow`, in `mutate` at `crates/adtb-testkit/src/corpus.rs:331`, where a parsed integer token is incremented by a picked offset with no bound. The same test takes 199s under deep, close to the kill timeout.

```
cargo nextest run --profile deep -p adtb-testkit --test differential corpus_mutants_equal_reference
```
## 2026-10-26: quality review

The overflow happens in `make`, outside `catch_unwind`, so it aborts the whole sweep rather than failing one case. Seeds 96 to 496 of the corpus sweep hit it.

## 2026-10-26: done

`mutate` in `crates/adtb-testkit/src/corpus.rs` moves an integer with `checked_add` and leaves the token alone when the move overflows. Test: `corpus::tests::moving_the_largest_integer_keeps_it_an_integer`. A panic in `make` still aborts the whole sweep, since `sweep_cases` calls it outside `catch_unwind`.

With the overflow gone, deep seeds 105, 162, 248, 311, 366, 376, 437, 455, 458 and 462 failed on a NaN row both sides gave: `within` in `differential.rs` compared floats with `==`. A NaN now agrees with a NaN (`a_nan_agrees_with_a_nan`). Seeds 96 to 496 pass under the deep profile in 216 s.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the fbug track.
