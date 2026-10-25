---
title: "Whether a temp relation may have a generated key"
pick: one
raised-by: [spec/temp-relations/txn]
options:
  none:
    title: "No: no type names a temp relation, so its key is plain fields or its whole row"
  temp-key:
    title: "Yes: its declaration also declares Key NAME for its rows"
---
Raised by the ad hoc statement checker on interop.md §4.3, whose
staging example declared `key line generate sequence` with a field
`line: Key identity_stage`. A generated key field has type `Key r` for
its own relation, and a temp relation is not in the schema any type
can name.

## spec/temp-relations/generated-key/none

A temp relation's `key` clause names plain fields and has no
`generate`; without a clause its whole row is its key:

```adt fragment
type StageRow = { line: I64, user: I64, kind: Text }
temp relation stage : StageRow key line
```

## spec/temp-relations/generated-key/temp-key

The declaration brings a type `Key stage` into scope for the text, and
`generate sequence` fills it. Needs types scoped to one ad hoc text,
which nothing else has, and a key type no row outside the text can
hold.
