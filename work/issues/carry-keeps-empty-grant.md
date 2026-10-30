---
created-at: 2026-10-29
status: open
kind: bug
priority: 570
summary: A carry that leaves a grant no covered field keeps it and prints ()
---

# A carry that leaves a grant no covered field keeps it and prints ()

## Report

Lead ruling 2026-10-29 (spec revision 30, N3): a grant that a carry leaves with no covered field
is dropped by that migration. Witness: `grant update identities.OAuth (provider)`, then a split
by `provider`; no product keeps a covered field. The carry code under crates/adtb-lang/src/evolve/
keeps the row and grant_text.rs prints `()`, which the grammar cannot parse. Fix: drop the row in
the carry; test with that split.
