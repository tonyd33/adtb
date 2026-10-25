---
title: Heap pages are reached through TIDs in index entries, not from one parent
pick: any
raised-by: [layout/checksum/in-parent]
options:
  heap-directory:
    title: Each heap is a tree of directory pages over its data pages
  hybrid:
    title: Tree pages are checked from their parent and heap pages carry an in-page checksum
  live:
    title: Live with checksums only where a parent exists
    excludes: [layout/checksum/no-parent/heap-directory, layout/checksum/no-parent/hybrid]
---
A union index entry `(key, ctor, tid)` names a heap page directly; many
entries name one page, and no entry can be rewritten each time the page
changes.

## layout/checksum/no-parent/heap-directory

A TID's page number is a logical index into the heap's directory; the
directory entry holds `(address, checksum)` and is rewritten with its
page, as in a copy-on-write tree. Every heap access adds a directory
lookup, usually cached.

```
heap identities.Google
  directory page:  [(addr, u128 checksum)] per logical page
  tid (page 17, slot 4) -> directory[17] -> block addr, verify, slot 4
```

## layout/checksum/no-parent/hybrid

B-tree and catalog pages verified from their parents; heap pages self
checked (`layout/checksum/in-page-32`), losing lost-write detection for
heap pages only.

## layout/checksum/no-parent/live

Heap pages go unchecked.
