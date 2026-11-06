# WAL-G

kind: system
home: https://github.com/wal-g/wal-g

## Archiving

- WALG_PREVENT_WAL_OVERWRITE: wal-push checks whether the WAL exists before uploading, and exits nonzero when a different file is archived under that name, so PostgreSQL keeps the segment. [V 2026-11-05] https://github.com/wal-g/wal-g/blob/master/docs/PostgreSQL.md
- In daemon mode failed uploads are not acknowledged and are retried after a short delay; a standalone wal-push rediscovers failed uploads on its next directory pass. [V 2026-11-05] same page
