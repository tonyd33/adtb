---
title: PostgreSQL heap MVCC, HOT, hint bits and wraparound
---
23-byte tuple header (xmin, xmax, cid, 6-byte ctid, infomasks, hoff);
ctid links old to new; a moved-partition ctid is a magic value with no
destination. HOT: an update touching no indexed column and fitting on
the page gets no index entries; page pruning redirects the root line
pointer. Hint bits cache commit status in the header and dirty pages on
read; with checksums the first hint after a checkpoint writes a full
page image. 32-bit xids wrap at 2^31, so every tuple must be frozen by
vacuum; 2-bit clog per xid. Visibility map (all-visible, all-frozen)
lets index-only scans skip heap fetches.
https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/heap/README.HOT
https://www.postgresql.org/docs/current/routine-vacuuming.html
https://wiki.postgresql.org/wiki/Hint_Bits
