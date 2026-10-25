---
title: Where time-zone rules come from
pick: one
raised-by: [model/time]
options:
  catalog:
    title: The rules are versioned catalog data; adopting a new release is an operation
  bundled:
    title: The engine binary carries one release
  host:
    title: The host's tzdata is read at run time
---
`to_instant(d, "Europe/Lisbon")` depends on the rules, and IANA
publishes several releases a year. A stored `ZonedDateTime` or a
domain check through a zone changes meaning when they change.

## types/tzdb/catalog

The release in force is part of the schema version
(`surface/version/counter`); a registered query's results are a
function of data and version. An upgrade states which stored future
times move.

## types/tzdb/bundled

Upgrading the engine changes conversions.

## types/tzdb/host

Postgres with `--with-system-tzdata`
(https://www.postgresql.org/docs/current/install-make.html, unverified);
replicas on different hosts can disagree.
