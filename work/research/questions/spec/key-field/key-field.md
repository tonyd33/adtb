---
title: "Whether the key field is declared by the type"
pick: one
options:
  declared:
    title: "The type declares the key fields; a self-key field is typed Key r and binds the type to r"
  added:
    title: "key id adds an engine field the type lacks"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/key-field/declared

The type declares the key fields; a self-key field is typed Key r and binds the type to r. An added field cannot belong to a type that serves several relation variables.

## spec/key-field/added

key id adds an engine field the type lacks. Select of the element type would drop the key and merge rows.
