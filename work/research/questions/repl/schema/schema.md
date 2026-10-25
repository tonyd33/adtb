---
title: How staged schema states and reader tracking reach replicas
pick: some
raised-by: [[evolution/old-readers/readers-first, repl/unit/physical], [evolution/old-readers/readers-first, repl/unit/change-set]]
options:
  in-log:
    title: Operators and stage transitions are log records applied in order with the data
    links:
      - {rel: gains, to: surface/source/op-log}
  forward-leases:
    title: A replica forwards bundle lease renewals and connections to the primary
    links:
      - {rel: gains, to: surface/reader-tracking/dormant/lease}
  replica-gate:
    title: The primary counts a replica as a reader at the version it has applied
  translate-at-replica:
    title: A replica serves a pinned older bundle through operator inverses
    needs: [surface/version-serving/translate]
---
Readers-first enables `Apple` for writes once no reader would meet it
untyped. Replicas, their connected bundles, and CDC consumers are
readers the primary does not see directly. The catalog is relations
and the schema is the fold of a linear operator log, so a replica can
receive both through the same stream.

## repl/schema/in-log

`add constructor Identity.Apple`, then `enable identities.Apple` are
commits like any other; a replica applies the add before any `Apple` row
since the log is ordered. Postgres logical replication lacks this and
stops when rows no longer fit the subscriber's schema.

```
0/1A00  op 0009 add constructor Identity.Apple { sub: Text }   -- state declared
                add unique (sub) in identities.Apple
0/1A40  op 0010 enable identities.Apple
0/1A41  changeset { identities.Apple + { ... } }
```

## repl/schema/forward-leases

A client connecting at a replica renews its bundle's lease, which is a
catalog write, so the replica forwards it. Without this, a client only
ever seen by replicas expires and is retired.

## repl/schema/replica-gate

`enable` waits until every replica has applied the add, so the
stage transition never outruns a replica that serves bundles pinned
to the new version. Lagging replicas delay enabling.

## repl/schema/translate-at-replica

The replica runs the same translation the primary would; it needs the
whole operator log, which `in-log` provides.
