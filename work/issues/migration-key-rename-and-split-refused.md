---
created-at: 2026-11-03
status: open
kind: bug
priority: 300
summary: "a migration is wrongly refused when renaming a composite key field reorders the key, or when splitting a constructor a stored key row holds"
---

# Migrations refused over composite key rename and split in a key

## Report

Found by the translate-reach track, reproduced there (receipt work/briefs/receipt-translate-reach-r1.md, local).

- Renaming a composite key field so that the key's sorted field order changes (prices keyed `(sku, n)`, `rename field Price.sku to code`) refuses the migration: "a value stored as Scalar(I64) does not convert to Scalar(Text)". Cause: crates/adtb/src/catalog/change.rs `convert` matches key fields by position. Same class as pinned-composite-key-order: identity by position in the key record.
- Splitting `Tier.Paid` when a stored row of `p`, keyed `(t: Tier, n)`, holds `Paid` in its key refuses with "p: a stored value holds constructor 1026 ...". Seen with a live bundle; not isolated without one.

Both are wrong refusals: no data changes and nothing wrong is returned, but a valid migration cannot apply.
