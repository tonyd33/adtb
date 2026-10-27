---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "a bundle live from an earlier open and given again is not a reader while the open's migrations run, so a migration that drops a constraint it reads applies at once"
tags:
  - engine
---

# Bug: earlier-live bundles are not readers during open's migrations

## Report

`docs/design/schema-change.md` (the registrant obligations) makes every live bundle a catalog reader whenever a migration runs, except one the running open recorded and has not yet registered. The engine never calls `Catalog::register` for bundles: `bundle::record` (`crates/adtb/src/api/bundle.rs`) skips a bundle that was live from an earlier open, and nothing registers it as a reader. A migration in the same open that drops a constraint or enables a constructor the bundle reads then applies at once instead of staying pending or being refused.

Ruling: before an open's migrations run, it registers as a reader every bundle it is given that was live before this open, at that bundle's pin. Only bundles the open records for the first time are exempt, as the design file says. Write the test first: open with bundle B, reopen giving B and a migration that drops a constraint B's query reads, and check the drop stays pending.
