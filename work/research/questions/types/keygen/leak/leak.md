---
title: Time-ordered ids reveal creation time and volume
pick: any
raised-by: [types/keygen/uuidv7, types/keygen/snowflake, types/keygen/sequence]
options:
  random-exposed:
    title: Keys shown outside the database use random ids
  permute:
    title: Ids leave the database through a keyed permutation
  live:
    title: Live with exposed creation order
    excludes: [types/keygen/leak/random-exposed, types/keygen/leak/permute]
---
A sequence leaks the row count; UUIDv7 and snowflake leak creation
time. RFC 9562 sec. 6.13 calls the timestamp a very small attack
surface.

## types/keygen/leak/random-exposed

Internal key UUIDv7, a second unique `public_id: Uuid = generate uuidv4`.

## types/keygen/leak/permute

A block cipher over the 64- or 128-bit id, applied in the client
codegen (`surface/codegen/schema-types`).

## types/keygen/leak/live

Accept it.
