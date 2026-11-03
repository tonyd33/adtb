---
created-at: 2026-10-31
status: resolved
closed: 2026-11-03
kind: bug
priority: 412
summary: a large store insert spins in Space::append when a fault overwrites a directory page
---

# Space append spin

## Report

From lost-bitmap. With a bitmap zeroed in place, the tenth of 40 inserts spun in `Space::append`/`add_page` under `LargeStore::insert`: the reused page was the large store's directory, so the loop never ends. The lost-bitmap fix closes this route. Any other fault that overwrites a directory page with a page of another kind can still spin there.

Direction: a kind check on a directory node refuses and marks. Done when a test overwrites a directory page with another kind and the insert answers `Quarantined` in bounded time.

## 2026-10-31: from the lost-bitmap review

Reachable with an intact but wrong bitmap, as after a lost write: `pager.free` of the large store's directory page, then one large insert gave no answer in 60 s, no mark. `add_page` returning `None` (heap/dir.rs:771) has no bound, so the loop in `append` never ends. `LargeStore::insert`'s `continue` (heap/large.rs near 190) can also loop on a foreign page.

## 2026-11-03: landed

Closed by `fix: refuse a foreign page in a heap directory instead of spinning`. Every directory walk in crates/adtb/src/heap/dir.rs reads through `Space::node`/`node_mut`, gated by `shaped` (kind, owner, capacity, count, level and capacity against the parent); a failing page is recorded damaged and the statement answers `Quarantined`. `find` checks the page it returns belongs to the space at that position. Holdings map large-store pages to the database. Search: crates/adtb-sim/tests/directory_damage.rs. Check exit 0, 3624 passed. Follow-ups on heap-directory-follow-ups.
