---
title: PostgreSQL unique index checks
---
An inserter finding an entry with an equal key fetches the heap tuple;
if its inserter or deleter is in progress it waits for that transaction
and rechecks. Entries for dead versions are skipped, so one key may
have several entries, at most one live.
https://www.postgresql.org/docs/current/index-unique-checks.html
