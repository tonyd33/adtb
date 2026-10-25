---
title: SQLite file format
---
One big-endian file of 512 B to 64 KiB pages with a 100-byte header.
B-tree pages carry an 8 or 12-byte header and a 2-byte cell pointer
array. A record is a varint header of serial types (one per column,
encoding type and length together) then values. Payload past a
threshold spills to a linked chain of overflow pages. Free pages hang
off trunk pages; auto-vacuum adds pointer-map pages.
https://www.sqlite.org/fileformat2.html
