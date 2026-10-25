---
title: Dropping a constructor frees pages inside the file but does not shrink it
pick: any
raised-by: [layout/files/single-file, layout/files/block-grid]
options:
  relocate:
    title: Move live pages down and truncate the file
  reuse-only:
    title: Freed pages go to the free list and the file never shrinks
    excludes: [layout/files/shrink/relocate]
---
Removing a constructor (settled evolution operator) drops a whole heap.
With one file its pages become holes.

## layout/files/shrink/relocate

SQLite auto-vacuum keeps pointer-map pages (5-byte entries: type,
parent page) so a page can be moved and its parent re-pointed, then the
file truncated. Heap pages are named by tids in index entries, so
moving a heap page rewrites every union index entry into it.
https://www.sqlite.org/fileformat2.html

## layout/files/shrink/reuse-only

Space returns to the allocator for later heaps; a full rewrite
(SQLite `VACUUM`) is an explicit offline operation.
