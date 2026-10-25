---
title: What an external system consuming changes receives
pick: any
raised-by: [write/primitive/delta]
options:
  change-stream:
    title: The committed change sets of chosen constructor relations, in commit order
    links:
      - {rel: prior-art, to: prior/repl-debezium}
      - {rel: same-as, to: repl/unit/change-set}
  query-delta:
    title: A registered query whose result deltas are streamed
    links:
      - {rel: prior-art, to: prior/constraint-ivm}
  registered-consumer:
    title: A consumer is a registered bundle, so readers-first gates on it
    needs: [surface/reader-tracking/bundles]
---
Debezium reconstructs row events from Postgres logical decoding and
cannot emit DDL. Here the write primitive already is a set of inserted
and deleted relations per constructor, and schema changes are typed
operators in the same log.

## repl/cdc/change-stream

```
subscribe identities_feed to identities.(Google | Apple)
-- each event: { position, xid, ctor, inserted: [...], deleted: [...] }
```

Sum values in payloads use the stable wire tag (the never-reused
constructor id), so a consumer decodes a constructor added later as a
known-unknown only if it is a registered reader.

## repl/cdc/query-delta

```
query active_google () -> Set { user: Key users, gid: Text } =
  from g in identities.Google, u in users
  where g.user == key u and u.active
  select { user: key u, gid: g.gid }
subscribe active_google deltas
```

Incremental view maintenance over the change stream (DBSP, Gupta and
Mumick); the consumer never sees constructors its query does not
name.

## repl/cdc/registered-consumer

A Kafka sink with compiled decoders is an old reader. Registering its
subscription as a bundle pinned to a schema version makes `enable
identities.Apple` wait for it, or retire it, like any client.
