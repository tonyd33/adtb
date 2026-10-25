---
title: Postgres extensions and aggregates
---
An extension is a control file plus versioned install and update scripts, chained by ALTER EXTENSION UPDATE, with an optional shared library loaded into the server. Aggregates are declared by state type, step, optional final, and combine/serialize functions for parallel partial aggregation.
https://www.postgresql.org/docs/current/extend-extensions.html
https://www.postgresql.org/docs/current/xaggr.html
