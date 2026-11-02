---
created-at: 2026-11-02
status: open
kind: bug
priority: 420
summary: "dropped whole-row uses let enable pass against the spec, an overwritten with field over-counts, and the read search costs 18 s by default"
---

# Read occurrence gaps

## Report

Left by `fix: keep reads a normal form eliminates`, whose review found no blocker. Answers stay right in each case.

- `rows` stays normal-form only, so `ignored_row(m)`, a dead `m == m` and `{a: m, b: 8}.b` let `enable` pass against docs/spec/evolution.md:1227-1229 and 1317-1319. Run by the review.
- `Mark { p with tone: Ink {...} }.tone` counts a read and blocks `enable`.
- No test pins a dead `Rel` or `Deref` blocking `enable r.C`, and deleting the `Item::Fn` walk at `crates/adtb/src/api/registered/check.rs:55-58` is predicted to survive.
- `each_binder_with_each_access_of_tone_is_stale_after_enable` takes 17.9 s by default. Removing `upsert-old` from `both_paths` saves an estimated 11 s, since the walk ignores binders and `every_case` keeps the full product.
- `crates/adtb/src/api/reads.rs:12` is a stale doc and the precondition at `:48` is unenforced.

