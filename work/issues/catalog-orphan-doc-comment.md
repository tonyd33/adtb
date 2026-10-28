---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 767
summary: "oversized was inserted under ctor_relations' doc comment, so it carries two docs and ctor_relations none"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# oversized carries ctor_relations' doc comment, and ctor_relations has none

## Report

`crates/adtb/src/catalog/mod.rs:270-273`: the new `fn oversized` was inserted between `ctor_relations`'s doc comment and its body, so `oversized` has two stacked "Returns" docs and `ctor_relations` (mod.rs:295) has none.

Fix: move the first two doc lines back above `ctor_relations`.

## 2026-10-27: resolved

`23024b1b` restores ctor_relations documentation and moves oversized with its own row-size contract into catalog/migration.rs.

Independent review found no blockers. Combined main check exited 0:
2329 root tests passed (8 skipped), 27 campaign tests passed (1 skipped),
root/campaign clippy and fmt, dependency policy and size checks passed.
House checks passed. Evidence: target/integrated-maintenance-check.log
and work/briefs/review-batch-final.md.

## 2026-10-27: resolved
