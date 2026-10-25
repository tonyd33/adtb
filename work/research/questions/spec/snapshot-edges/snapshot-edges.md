---
title: "Conflicts with rows committed after the snapshot"
pick: one
options:
  serialization:
    title: "A conflict, wait or key violation against such a row is a serialization failure; no re-evaluation on successors"
  reevaluate:
    title: "Waiters re-evaluate on the newer version"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/snapshot-edges/serialization

A conflict, wait or key violation against such a row is a serialization failure; no re-evaluation on successors. Re-evaluation mixes post-snapshot rows into a snapshot statement.

## spec/snapshot-edges/reevaluate

Waiters re-evaluate on the newer version. Not serializable.
