---
title: "The key of a temp relation of a sum with several constructors"
pick: one
raised-by: [spec/temp-relations/txn]
options:
  declared:
    title: "It must declare its key; one without a key clause is refused"
  whole-row:
    title: "Without a key clause, its whole row, constructor included, is its key"
---
Raised by the temp relation checker. A temp relation without a `key`
clause is keyed by its whole row, and a relation's key is a list of
fields every constructor has.

## spec/temp-relations/sum-key/declared

A temp relation of a sum with several constructors declares its key:

```adt fragment
temp relation ids : Identity key id
temp relation ids : Identity          -- refused: a sum of several constructors declares its key
```

## spec/temp-relations/sum-key/whole-row

The whole value, constructor and fields, is the key. Needs a key that
is not a list of fields, which no other relation has.
