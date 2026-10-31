---
created-at: 2026-10-31
status: open
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
