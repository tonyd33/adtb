---
title: "Whether an insert of an existing key replaces the row"
pick: one
options:
  violation:
    title: "An insert never replaces; a pre-state key not deleted by the change set is a KeyViolation, whatever the value"
  equal-noop:
    title: "An insert of a row equal to the pre-state row is a no-op; a different row is a KeyViolation"
  replace:
    title: "An insert of an existing key replaces the row"
---
Raised by the implementability review of docs/spec/. writes.md §1.4
said only that two different inserted rows with one key fail; an insert
whose key already has a row in the snapshot, with the same or another
value, had no stated outcome, and normalisation made an equal insert
vanish silently.

```adt
-- users holds { id: users.key(1), name: "a" }
insert users { id: users.key(1), name: "a" }
```

## spec/writes/insert-existing/violation

A key of the inserted rows that is the key of a pre-state row not
deleted by the same change set fails the statement with
`KeyViolation`, even when the new row equals the old. An upsert's
proposed row that clashes with another write's insert of the same
statement is the same violation. A delete and an insert of one key in
one change set remain a replacement.

## spec/writes/insert-existing/equal-noop

An insert of a row equal to the pre-state row succeeds and changes
nothing; only a different row is a violation. Then the inserted rows a
write reports include rows the change set does not hold, contradicting
writes §1.2, and whether an insert fails depends on the old value.

## spec/writes/insert-existing/replace

An insert of an existing key replaces the row, as an implicit upsert.
Silently overwrites data and duplicates the upsert form.
