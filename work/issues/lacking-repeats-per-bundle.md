---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 840
summary: "opening and a migration recheck normalise a program once per nonowner bundle, and lacking_of rebuilds the effective grants per callable"
closed: 2026-10-30
tags:
  - quality
  - engine
---

# Opening and a migration recheck normalise a program once per bundle

## Report

registration-normalises-per-callable moved the normalise cache into `lacking_of` (`crates/adtb/src/api/registered.rs:300`), so it lives for one bundle. `recheck` calls `lacking_of` once per live nonowner bundle (`registered.rs:730`), each over the same `schema` and `program` Arcs of the migration's result, so `norm::program` runs once per such bundle over one program. `definers` does the same at open through `lacking` (`registered.rs:361`) for bundles pinned at one version. The ticket asked for once per `Over` and per translation target.

`EffectiveGrants::of` (`registered.rs:318`) sits in the same loop and is rebuilt per callable, though it depends only on the schema, the grants and the role. Each build walks the role's grants, and `GrantTarget::ctors` scans every constructor id of the schema for each one (`crates/adtb-lang/src/evolve/grant.rs:42`).

Fix: hold the cache in the caller across bundles, keyed as it is now by `Arc::ptr_eq`, and keep the `EffectiveGrants` in the same entry.

## 2026-10-30: resolved
