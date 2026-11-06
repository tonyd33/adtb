# pgBackRest

kind: system
home: https://pgbackrest.org/

## Archiving

- An archived WAL segment's repository name is the segment name, a dash, a 40-hex-digit hash and the compression suffix, e.g. `000000010000000000000001-42e768a79e0022564a2c378e7beb646154378858.gz`; the guide does not say what the hash is computed over. [V 2026-11-05] https://pgbackrest.org/user-guide.html
- archive-push-queue-max: past the limit pgBackRest tells PostgreSQL the WAL was archived and drops it, logging a warning; the archive stream is broken, PITR past that point is lost and a new backup is needed. In async mode the whole queue is dropped, and the limit is checked only at the start of each push. [V 2026-11-05] https://pgbackrest.org/command.html
- archive-check (default on): a backup waits until every WAL segment its consistency needs is in the archive. [V 2026-11-05] same page
- With several repositories, WAL still goes to the reachable ones; WAL reaching none stays in pg_wal, which may fill. [V 2026-11-05] https://pgbackrest.org/user-guide.html
- The async spool path holds archiving status files, all recreatable. [V 2026-11-05] same page
