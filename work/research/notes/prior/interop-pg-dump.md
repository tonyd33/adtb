---
title: pg_dump
---
Plain, custom, directory (parallel) and tar formats. Sections pre-data (definitions), data, post-data (indexes, triggers, most constraints), so restore loads rows before building indexes and checking FKs. Extensions dump as CREATE EXTENSION, not their members.
https://www.postgresql.org/docs/current/app-pgdump.html
