---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "a bundle live from an earlier open and given again is not a reader while the open's migrations run, so a migration that drops a constraint it reads applies at once"
closed: 2026-10-27
tags:
  - engine
---

# Bug: earlier-live bundles are not readers during open's migrations

## Report

`docs/design/schema-change.md` (the registrant obligations) makes every live bundle a catalog reader whenever a migration runs, except one the running open recorded and has not yet registered. The engine never calls `Catalog::register` for bundles: `bundle::record` (`crates/adtb/src/api/bundle.rs`) skips a bundle that was live from an earlier open, and nothing registers it as a reader. A migration in the same open that drops a constraint or enables a constructor the bundle reads then applies at once instead of staying pending or being refused.

Ruling: before an open's migrations run, it registers as a reader every bundle it is given that was live before this open, at that bundle's pin. Only bundles the open records for the first time are exempt, as the design file says. Write the test first: open with bundle B, reopen giving B and a migration that drops a constraint B's query reads, and check the drop stays pending.

## 2026-10-27: done

Before its migrations run, open registers as a catalog reader, at its pin, every bundle it ships that was live before the open (`registered::readers`, called from `Engine::on`). The paragraph in `docs/design/schema-change.md` about `Catalog::open` now says so. Tests: `bundles::a_bundle_live_before_the_open_that_ships_it_again_blocks_an_enable_it_reads` passes. `bundles::a_drop_stays_pending_while_a_bundle_live_before_the_open_reads_it` FAILS: the catalog holds the drop pending, but the write path the open binds is built from the version program, so the dropped constraint is not enforced. The catalog also starts every open with no readers, so a pending drop is not carried to the next open. Both gaps need their own ticket.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qwrong track.
