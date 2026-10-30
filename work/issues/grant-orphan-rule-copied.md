---
created-at: 2026-10-30
status: in-progress
branch: server-fixes
kind: bug
priority: 640
summary: "the rule for which rows a lost grant option orphans lives in server orphans and catalog refuse_orphans, over two row shapes, and they already read differently"
tags:
  - quality
  - engine
---

# The rule for which rows a lost grant option orphans is written twice, over two row shapes

## Report

Whether a grant option's loss orphans a row, and which grant options build the cycle graph, are decided in two places:

- `crates/adtb/src/api/server/mod.rs:362` `orphans` and `:397` `closes_cycle`, over `Grant`, for a deployment revoke and grant.
- `crates/adtb/src/catalog/grants.rs:166` `refuse_orphans`, over `Stored` rows read through `Shape`, for a migration.

Each repeats the same test: same grantee as the row's grantor, same privilege, holds the option, `target_covers`, grantor not the grantee. They already read differently: `orphans` skips a dormant survivor, and the deployment half of `refuse_orphans` (`grants.rs:202`) has no dormant test. Neither path's tests would see a change to the other.

Fix direction: state the rule once in `adtb_lang::evolve` beside `target_covers` and `closes_option_cycle`, over a small row view like `OptionEdge`, with both callers mapping their rows onto it. Test the rule there, including the dormant case.
