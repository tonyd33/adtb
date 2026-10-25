---
title: fsync failure semantics (fsyncgate 2018, Rebello et al. ATC 2020)
---
Linux file systems mark pages clean after a failed writeback, so a
retried fsync can succeed without the data reaching disk; ext4 and XFS
keep the unwritten content in cache, Btrfs reverts it. Postgres now
PANICs and replays WAL. Rebello et al. found every studied application
(PostgreSQL, LMDB, LevelDB, SQLite, Redis) can lose or corrupt data,
and that recovery read through the page cache sees content never
written. FreeBSD re-dirties failed pages.
https://wiki.postgresql.org/wiki/Fsync_Errors
https://www.usenix.org/system/files/atc20-rebello.pdf
