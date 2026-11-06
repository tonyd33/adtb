---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 350
summary: "a directory leaf whose write was lost can name a page vacate freed, and writers insert into it while the allocator hands it to another object"
closed: 2026-11-06
tags:
  - class-damage-unreported
---

# A stale directory leaf lets writers use a freed page

## Report

## 2026-11-05

From the damage-reporting audit, reasoned, not reproduced: prune empties page P, vacate writes leaf L, L's write-out is lost, reopen. `Space::misnamed` (heap/dir.rs:723-744) and `candidate`/`ours` check only the header's owner and position, never the bitmap; the freed page keeps its old header, so it reads `Named::Held` and inserts land in a page the allocator may give another object. Reproduce first. Covered at the source by damage-readin-gate; a bitmap check in `misnamed` is the local fix. Also: `refuse` (dir.rs:407-415) blames a stale leaf as Checksum, not Stale. Defect class: Damage reported as nothing.

## 2026-11-06: fixed by the read-in gate

Fixed in 0d1250ce (damage-readin-gate), reproduced with the gate off. `a_directory_page_that_lost_the_write_freeing_a_page_is_refused_on_its_first_read` in crates/adtb-sim/tests/sim/verify.rs: the stale leaf is refused Stale on its first read, so no writer inserts through it. Not addressed: `refuse` in heap/dir.rs blaming a leaf it meets as Checksum rather than Stale; the gate refuses a stale leaf before it is reached.

## 2026-11-06: resolved
