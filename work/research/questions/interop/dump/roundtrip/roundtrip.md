---
title: What a dump and restore preserves
pick: one
raised-by: [interop/dump/logical]
options:
  same-ids:
    title: Equal relations, the same schema version, constructor ids and key values
  equal-relations:
    title: Equal relations under the current schema; ids and version may be renumbered
---
Stable constructor ids are wire tags (`surface/sum-encoding/stable-tag`)
and stored tags (`layout/tag-value/stable-ordinal`); bundles pin the
schema version.

## interop/dump/roundtrip/same-ids

A deployed client keeps working against the restored database. Engine
generated key fields restore as written, and generators resume past
them.

## interop/dump/roundtrip/equal-relations

A restore is a fresh database with the same contents; every client
bundle re-registers.
