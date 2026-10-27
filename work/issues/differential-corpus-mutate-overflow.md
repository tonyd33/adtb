---
created-at: 2026-10-26
status: open
kind: bug
priority: 699
summary: differential corpus_mutants_equal_reference overflows under the deep profile with attempt to add with overflow in mutate
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
