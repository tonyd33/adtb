---
title: Postgres LISTEN/NOTIFY and logical decoding
---
NOTIFY: channel plus payload up to 8000 bytes, delivered at commit in
commit order, folded within a transaction, sent only between
transactions. Logical decoding: WAL decoded per replication slot in
change order; a slot retains WAL until consumed.
https://www.postgresql.org/docs/current/sql-notify.html
https://www.postgresql.org/docs/current/logicaldecoding-explanation.html
