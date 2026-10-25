---
title: Replication around embedded SQLite
---
Litestream: async WAL shipping to object storage as TXID-numbered LTX
files plus daily snapshots; PITR. LiteFS: FUSE capture into LTX,
primary by Consul lease, async, rolling whole-database checksum.
dqlite: WAL frames through Raft. rqlite: SQL statements through Raft,
read levels none to linearizable. libSQL embedded replicas: local
file synced from a primary, writes forwarded, read-your-writes for the
writer only. Cloudflare D1: async read replicas, sessions carry a
bookmark for sequential consistency; Time Travel restores any minute
of 30 days. mvSQLite: SQLite pages on FoundationDB.
https://litestream.io/how-it-works/ https://docs.fly.io/litefs/how-it-works
https://docs.turso.tech/features/embedded-replicas/introduction
https://developers.cloudflare.com/d1/best-practices/read-replication/
