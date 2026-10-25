# Graefe, Modern B-Tree Techniques

kind: paper
home: https://doi.org/10.1561/1900000028

## Keys

- Foundations and Trends in Databases 3(4), 2010 (c 2011), pp 203-402. [V 2026-10-25] https://citeseerx.ist.psu.edu/document?repid=rep1&type=pdf&doi=0b19f413ffb5bc68b43f3bd05a97c282a7c6d6ab
- Normalized keys: multi-column keys transformed to one binary string so memcmp orders them; a leading bit per column marks null (0 sorts low); strings need a terminator, not a length prefix, else binary order breaks; if the terminator can occur, the alphabet needs one more symbol. Lossy collations (case-insensitive) need the original value stored too, or normalized keys only in branch nodes. Normalized keys can be longer than originals. [V 2026-10-25] same, sec 3.4
- Prefix truncation stores a node's common prefix once; basing it on the fence keys (copies of the separators bounding the page) rather than the actual keys means an insert never forces reformatting. Dynamic prefix truncation: bytes shared by the two separators flanking a child need not be compared below. Suffix truncation picks the shortest separator between the two keys at the split point, choosing among acceptable split points the one giving the shortest separator. [V 2026-10-25] same, sec 3.5
- Offset-value coding replaces a shared prefix with its length relative to the predecessor; suits scans and merges, not binary search. [V 2026-10-25] same, sec 3.5
- Duplicates: all B-tree entries must be unique (ambiguity on deletion and index-to-row navigation); representations include key per RID, key with RID list, RID list with counters, bitmaps; RID lists kept sorted, delta-compressed. [V 2026-10-25] same, sec 3.7
- Ghost records: delete sets one bit in the record header; queries skip ghosts; key stays locked until commit so rollback cannot fail on space and no other txn inserts the key; removal deferred to a later insert or compaction. [V 2026-10-25] same, sec 4.2
- B-link trees split in two independent steps: split node with link to right neighbor, then post separator in parent later (any later traversal, even after a crash); split is local, latching only the overflowing node; "foster parent" state. [V 2026-10-25] same, sec 4.6
- Härder's combined image: one key, occurrence counters per table, then the record IDs per table (join index over several tables). [V 2026-10-25] same, sec 5.8
- Merged indexes: one B-tree interleaving records of several indexes by a common sort order (master-detail clustering); normalized keys alternate domain tags and field values up to and including the index identifier; tag can share a byte with the null indicator. [V 2026-10-25] same, sec 7.3
