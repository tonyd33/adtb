---
title: Postgres index access methods and GiST
---
An index AM is a table of callbacks (build, insert, scan, bulk delete, vacuum cleanup, cost estimate). GiST is a framework over it where the type author writes consistent, union, penalty, picksplit and same; GiST owns concurrency, WAL and tree search. Extension AMs log through generic WAL records. An AM without predicate-lock support makes serializable scans lock the whole index.
https://www.postgresql.org/docs/current/indexam.html
https://www.postgresql.org/docs/current/gist.html
https://www.postgresql.org/docs/current/generic-wal.html
https://www.postgresql.org/docs/current/index-locking.html
