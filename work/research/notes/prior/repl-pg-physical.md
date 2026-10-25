---
title: Postgres streaming replication and hot standby
---
WAL shipped byte for byte; the standby replays it and serves strictly
read-only queries. Serializable is refused on a hot standby. Replayed
DDL locks and vacuum cleanup conflict with standby queries, which are
cancelled after `max_standby_streaming_delay` unless
`hot_standby_feedback` holds back the primary's vacuum.
`synchronous_commit` = off, local, remote_write, on, remote_apply.
https://www.postgresql.org/docs/current/hot-standby.html
https://www.postgresql.org/docs/current/runtime-config-wal.html
