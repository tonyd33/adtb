---
created-at: 2026-11-10
status: open
kind: bug
priority: 450
summary: "readers that scan heaps outside the executor's store read without any check that their hold's scope covers the heap"
tags:
  - class-admission-per-path
---

# Heap scans outside the store have no scope check

## Report

## 2026-11-10

structure-transactions step 5 made every reader carry its hold's scope and made the executor's store refuse a heap or index the reader does not cover. Readers that scan heaps directly, outside any store, check nothing: in the catalog `visible`, `delete_rows`, `replace_kept` and `migrations`; in the write path `apply::reader_as`, `reindex::run` and `verify::check_rows`. Each one's scope covers what it reads today (the catalog's ROOTS-only holds read only ROOTS; the rest hold every heap), so nothing is misread; a caller that later passes a narrower hold would read a heap whose watermark its snapshot does not hold back, and vacuum could remove versions it should see.

Direction: give the heap scan entry these readers use a check against `Reader::covers` that refuses like the store's (`Internal` naming the heap), so the rule has one gate; a test per kind of reader with a scope that omits the heap it reads.

Defect class: Authority and admission checked per entry path.
