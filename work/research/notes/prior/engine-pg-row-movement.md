---
title: PostgreSQL cross-partition UPDATE under concurrency
---
An UPDATE moving a row to another partition is a delete plus insert.
The old tuple cannot point at its successor, since a 6-byte TID names no
partition, so a concurrent updater or locker gets "tuple to be locked
was already moved to another partition due to concurrent update"
instead of following the update chain. Commit f16241b (Postgres 11).
https://github.com/postgres/postgres/commit/f16241bef7cc271bff60e23de2f827a10e50dde8
