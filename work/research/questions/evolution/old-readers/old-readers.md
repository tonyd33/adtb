---
title: Readers compiled before an added constructor meet its rows
pick: any
raised-by: [evolution/add]
options:
  unknown-ctor:
    title: Old readers see an unknown constructor value
    links:
      - {rel: same-as, to: query/unknown-arm}
  readers-first:
    title: Require readers to upgrade before writers use the constructor
  live:
    title: Live with old readers failing
    excludes: [evolution/old-readers/unknown-ctor, evolution/old-readers/readers-first]
links:
  - {rel: prior-art, to: prior/protobuf-avro}
---
Replicas, drivers and clients with compiled decoders receive an `Apple`
row. Avro signals an error for a writer branch the reader lacks.

## evolution/old-readers/unknown-ctor

As protobuf keeps unknown enum values: the reader gets an opaque
constructor it can pass through but not destructure.

## evolution/old-readers/readers-first

The constructor is declared, then enabled for writes once every
registered reader reports the new schema.

## evolution/old-readers/live

Readers fail until upgraded.
