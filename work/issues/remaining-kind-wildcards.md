---
created-at: 2026-11-02
status: open
kind: bug
priority: 130
summary: "Other Kind wildcards in translation, single form wildcards and Prim operators are not named"
---

# Remaining kind wildcards

## Report

The slice in `fix: deny wildcard arms over Kind in reader and translation analyses` found a wrong answer behind `generic`'s `Restrict` arm, so the unjudged wildcards deserve the same reading.

- crates/adtb-lang/src/translate/walk.rs: `kind`, `stored_whole`, `reads_probed`, `probed_rows`, `rows_of`, `yields`, and those at lines 67-129. `kind` and `stored_whole` delegate to `generic` and `carries`.
- `clippy::wildcard_enum_match_arm` does not fire on a wildcard covering exactly one form; `clippy::match_wildcard_for_single_variants` does and is not denied.
- `Kind::Prim(..)` arms in `compared` and `reads_of` do not name operators.
- Nothing at type level ties a constructor set to its mapping in `generic`.
- A `match` on a selection of a removed constructor builds a zero arm `Case`; `a_pinned_selection_reads_its_constructors_after_a_removal_moves_their_positions` reads only `x.n`. Add `match`.
