---
created-at: 2026-11-09
status: open
kind: bug
priority: 180
summary: "mapping a damage record walks every interior page of the LSN map, and a refused map page met on that walk is recorded as a side effect of mapping"
tags:
  - class-unbounded-work
---

# Bug: mapping a record walks the whole LSN map

## Observed

`holders_meeting` (crates/adtb/src/storage/quarantine.rs) asks `holds_map_page` (storage/lsnmap.rs) whether a page is an LSN map page, and that walks every interior page of the map for each record mapped: O(map pages) per damaged page. A refused map page met on the walk is recorded as damage while mapping another record.

## Expected

A page is classified as a map page from its header kind and owner, or from one descent, and mapping records no new damage.

## Reproduction

Read `holds_map_page`. Found in the review of structure-damage-policy step 4b.
