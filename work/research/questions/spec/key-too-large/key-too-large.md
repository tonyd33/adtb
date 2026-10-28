---
title: How an indexed value exceeding its encoded key limit is refused
pick: one
options:
  rejected:
    title: Reject the statement with KeyTooLarge naming the index and payload limit
  row:
    title: Report oversized index keys as RowTooLarge
  overflow:
    title: Store unbounded index keys out of line
---
An otherwise storable row can exceed an index's entry limit. A 20000-byte
text key currently reaches fatal TooLong on an 8192-byte page. This is an
observable input limit, independent of whether the row itself fits.

## spec/key-too-large/rejected

Add `KeyTooLarge { relation: Text, ctor: Text, index: Text, size: I64,
limit: I64 }` to rejected reasons. The relation is module-qualified and
index is its canonical catalog clause including constructor scope. Size
is the canonical encoded key payload's byte length, excluding constructor
and TID suffixes. Limit is the database-derived payload capacity for that
constructor. Equality fits. The statement rolls back and its transaction
remains usable. Migration index builds use SchemaRefused with the index,
size and limit.

Check normalized new rows after NotOpen and RowTooLarge, before uniqueness
and constraints or physical writes. Expression evaluation faults remain
Faulted; competing failures follow the existing any-abort freedom.

The exact physical capacity remains a storage contract, not an edition
constant. The current entry capacity at page size P is
floor((P - 42) / 5) - 12, less the constructor's ordered-varint width and
eight TID bytes for the payload. An 8192-byte page permits 1618 entry
bytes. The investigated constructor permits 1607 payload bytes.

## spec/key-too-large/row

Reuse RowTooLarge for index overflow. A caller sees a row-size failure
even when the heap representation fits, with no index identity.

## spec/key-too-large/overflow

Make index keys arbitrarily large through overflow storage. This changes
comparison, page traversal and storage recovery to remove this input limit.
