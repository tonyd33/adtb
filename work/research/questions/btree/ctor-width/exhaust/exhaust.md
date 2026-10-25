---
title: A fixed-width constructor id runs out under a counter that never goes back
pick: any
raised-by: [btree/ctor-width/u16, btree/ctor-width/local-code]
options:
  widen:
    title: Index format version bump and online rewrite to a wider field
  live:
    title: Live with a hard lifetime limit on constructors
    excludes: [btree/ctor-width/exhaust/widen]
---
Ids are never reused, so drop and re-add cycles, aborted DDL and
split operators consume them. `u16` caps a database at 65536 ever; a
one-byte `local-code` caps an index at 256 constructors ever added to
its set.

## btree/ctor-width/exhaust/widen

Each index page records its entry format; a rebuild writes the wider
format while old pages are still readable.

## btree/ctor-width/exhaust/live

Fail the DDL that would allocate past the limit.
