---
title: How sum migrations are specified
pick: some
options:
  functorial:
    title: Migrations as schema mappings with Sigma, Delta, Pi
    links:
      - {rel: prior-art, to: prior/cql}
  scripted:
    title: Hand-written migration scripts per change
  smo:
    title: A closed language of schema modification operators
    links:
      - {rel: prior-art, to: prior/prism-bidel}
  lens:
    title: Bidirectional lenses between schema versions
    links:
      - {rel: prior-art, to: prior/prism-bidel}
  read-resolve:
    title: Resolve old data against the reader's schema at read time
    links:
      - {rel: prior-art, to: prior/protobuf-avro}
---
How a change to a sum's constructors is turned into data movement.
Methods stack, for instance operators with scripts as the fallback.

## evolution/functorial

A migration is a mapping between schemas; data moves by the induced
Sigma (union-like, pushes forward), Delta (projection, pulls back) and Pi
(join-like). A merge is a Sigma along the map sending both constructors
to one; a split needs a routing function and is not a plain schema map
(unverified whether CQL expresses it directly).

## evolution/scripted

Each change ships with a script that moves the rows. The system
checks only that the result satisfies the new schema.

## evolution/smo

PRISM/BiDEL style: `PARTITION` is split, `MERGE` is merge, `DECOMPOSE`
and `JOIN` approximate move-field. Each operator has a known inverse or
is flagged not information preserving.

## evolution/lens

Cambria or BiDEL symmetric lenses: each change defines forward and
backward translations, so old and new versions coexist. Lenses cannot
look up data absent from the row.

## evolution/read-resolve

No data movement. Rows keep the schema they were written under; readers
resolve them against their own, as Avro does with writer and reader
schemas.
