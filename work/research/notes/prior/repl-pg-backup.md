---
title: Postgres base backup, archive and PITR
---
Fuzzy file copy plus continuous WAL archive; replay to a time, xid,
LSN or named point; a finished PITR starts a new timeline. PG 17 adds
block-level incremental backup from WAL summaries, combined with
`pg_combinebackup`. `pg_verifybackup` checks manifest checksums and
parses the needed WAL, and says it cannot catch everything recovery
would. https://www.postgresql.org/docs/current/continuous-archiving.html
https://www.postgresql.org/docs/current/app-pgverifybackup.html
