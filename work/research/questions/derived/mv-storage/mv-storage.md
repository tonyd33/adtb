---
title: How a stored derived relation is physically held
pick: one
raised-by: [derived/materialize/storage-choice, derived/materialize/refresh-kind]
options:
  heaps:
    title: Like a base relation, one heap per constructor with a union index
    links:
      - {rel: gains, to: storage/table-per-ctor}
  arrangement:
    title: A separate indexed structure owned by the maintainer
    links:
      - {rel: prior-art, to: prior/derived-materialize}
      - {rel: prior-art, to: prior/derived-dd}
---
A stored view whose result is a sum has constructors like any relation.

## derived/mv-storage/heaps

The maintainer writes the view's heaps through the ordinary write
path. MVCC versions it, so every snapshot reads the view at its own
version and vacuum, recovery and retag work unchanged.

## derived/mv-storage/arrangement

Materialize index: cluster-local, in memory, multiversioned by the
dataflow's own timestamps, rebuilt on restart. Readers need a second
read path and the snapshot mapping between versions and dataflow time.
