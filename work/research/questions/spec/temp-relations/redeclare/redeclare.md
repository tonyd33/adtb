---
title: "Whether a temp relation name may be declared twice in one transaction"
pick: one
raised-by: [spec/temp-relations/txn]
options:
  refused:
    title: "Refused, as any repeated declaration is"
  replaces:
    title: "The second declaration replaces the first and its rows"
---
Raised by the report on `spec/temp-relations/names`. A temp relation
lives until its interactive transaction ends, and several ad hoc texts
of that transaction may declare one.

## spec/temp-relations/redeclare/refused

Declaring a temp relation whose name a temp relation of the same
transaction already has is refused:

```adt fragment
temp relation stage : { line: I64 } key line
temp relation stage : { line: I64 } key line   -- refused: stage is declared
```

## spec/temp-relations/redeclare/replaces

The later declaration drops the earlier relation and its rows. A
script rerun in one transaction then needs no cleanup, but a typo
silently discards staged rows.
