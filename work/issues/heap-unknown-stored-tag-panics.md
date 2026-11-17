---
created-at: 2026-11-15
status: resolved
closed: 2026-11-17
kind: bug
priority: 200
summary: a heap row whose stored constructor tag names no constructor of its row type panics the engine instead of reporting damage
tags:
  - engine
  - class-failure-flattened
---

# Heap unknown stored tag panics

## Report

Found by the review of structure-lang-identity step 11 part e. `ctor_index` in crates/adtb/src/heap/row.rs maps a stored constructor tag to its position in the row type and panics ("stored tag {id} names no constructor") when no constructor has that id. The tag is read from a page, so a damaged page, a file written by a newer build with a constructor this build lacks, or a row type mismatch reaches a panic instead of a damage report naming the heap and row.

Expected: a stored tag that names no constructor is damage (a `Rot` of the heap row, with the tag and the row type), reported through the same path as other undecodable rows, so open, scrub and salvage see it.

Repro direction: write a roots row, change its tag byte on the page to an unused id, and read the heap.

Defect class: Damage or failure gives a panic, hang or flattened error.

## Resolution 2026-11-17

Fixed by `fix: report a heap row that does not decode as damage of its page`, part 1 of page-decode-returns-result. A stored tag naming no constructor decodes to `Undecoded::Tag`; the heap refuses the page as `Shape::Row(Tag)`, verify reports it, and a statement fails with `Fatal::Storage` naming the page. Covered by the search in crates/adtb/src/heap/row_damage_tests.rs.
