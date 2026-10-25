---
title: The partition unit if the database is ever spread across nodes
pick: one
options:
  never:
    title: Never partition one database; scale by replicas and many databases
  by-key:
    title: Partition every sum by a key prefix, all constructors of a key together
    links:
      - {rel: prior-art, to: prior/engine-distributed-sql}
  by-constructor:
    title: Place constructor heaps of one sum on different nodes
    assessment: {grade: costly, why: "the sum key, retag and every match span nodes, which is what one heap per constructor with a union index keeps local"}
  by-sum:
    title: Place whole sums on different nodes
links:
  - {rel: prior-art, to: prior/engine-distributed-sql}
---
`engine/distributed` was not chosen: the core is embedded with a
server around it. This question records which direction a later
scale-out would take, so nothing settled now forecloses it or
pretends to support it. Every index, unique constraint and FK target
ranges over constructors of one sum, and a retag moves a row between
heaps atomically.

## repl/distribution/never

Scale out by replicas and by many independent databases (one per
tenant, as D1 and Turso deploy, unverified). The spec states that one
database is one node's serial history.

## repl/distribution/by-key

Every sum partitioned on a key prefix; all constructors of a key live
together, so exclusivity and retag stay local. Unique constraints not
led by the partition key and FKs across partitions become
distributed checks (CockroachDB checks every region for REGIONAL BY
ROW uniqueness).

```
type Identity = EmailPass { ... } | Google { ... }
relation identities : Identity
  key id
  partition by (user)
```

## repl/distribution/by-constructor

Constructor heaps on different nodes. Every insert checks the sum key
on every node, a retag is a distributed transaction, and a match is a
union across nodes.

## repl/distribution/by-sum

Whole sums per node; queries joining sums and FKs between them cross
nodes.
