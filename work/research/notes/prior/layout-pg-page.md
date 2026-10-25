---
title: Postgres heap page and tuple header
---
8 kB pages: 24-byte header (LSN, 16-bit checksum, flags, lower, upper,
special, size/version, prune xid), 4-byte line pointers growing down,
tuples growing up. Heap tuple header 23 bytes (xmin, xmax, cid, 6-byte
ctid, two infomasks, hoff), then a null bitmap when any column is null.
Torn pages repaired from full-page images logged on first change after a
checkpoint. Values over about 2 kB go to a TOAST table in 2000-byte
chunks behind an 18-byte pointer.
https://www.postgresql.org/docs/current/storage-page-layout.html
https://www.postgresql.org/docs/current/storage-toast.html
https://www.postgresql.org/docs/current/wal-reliability.html
