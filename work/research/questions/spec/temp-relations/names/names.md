---
title: "Whether a temp relation may take a schema relation's name"
pick: one
raised-by: [spec/temp-relations/txn]
options:
  distinct:
    title: "No: the name must not be a relation variable of the schema"
  shadow:
    title: "Yes: the temp relation shadows the schema relation in the text that declares it"
---
Raised by the ad hoc statement checker. `temp relation NAME : T` names
a relation outside the log, and nothing said what happens when `NAME`
is also a relation variable of the schema.

## spec/temp-relations/names/distinct

A temp relation whose name is a relation variable of a module the text
reads is refused:

```adt fragment
temp relation users : { name: Text }   -- refused: users is a relation of the schema
```

A name in ad hoc text then means one relation throughout.

## spec/temp-relations/names/shadow

The temp relation hides the schema relation for the rest of the text.
Lets a script rename nothing, but a write meant for the staging copy
goes to the real relation when a statement runs before the declaration,
and a later migration adding a relation of that name silently changes
what old scripts read.
