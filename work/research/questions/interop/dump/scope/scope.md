---
title: What a logical dump carries besides schema and rows
pick: any
raised-by: [interop/dump/logical]
options:
  bundles:
    title: Registered bundles and their version pins
  extensions:
    title: References to installed extensions by name and version, not their contents
    links:
      - {rel: prior-art, to: prior/interop-pg-extensions}
  stats:
    title: Per-constructor statistics sketches
---
The catalog is relations (`engine/catalog-relations`), so each of
these is dumpable like data; the question is which a restore should
reproduce.

## interop/dump/scope/bundles

Registered queries restore registered; a restored server serves
deployed clients with no redeploy.

## interop/dump/scope/extensions

`pg_dump` emits `CREATE EXTENSION`, not member objects
(https://www.postgresql.org/docs/current/extend-extensions.html).

## interop/dump/scope/stats

Plans after restore match plans before; `pg_dump` dumps statistics
in its data section (https://www.postgresql.org/docs/current/app-pgdump.html).
