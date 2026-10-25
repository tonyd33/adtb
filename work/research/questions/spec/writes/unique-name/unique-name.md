---
title: "The canonical name of a unique"
pick: one
options:
  fields-or-id:
    title: "A field unique is named by relation, field paths and constructor scope; an expression unique is r.unique#n, n its catalog id"
  expression-text:
    title: "Every unique is named by its expressions as text"
---
Raised while applying the ruling that every constraint has a canonical
name. A unique over expressions was named by their text as declared,
which needs a canonical printing of expressions nothing else requires.

## spec/writes/unique-name/fields-or-id

A unique over field paths is named by its relation, its field paths
and its constructor scope. A unique with an expression among its
fields is named `r.unique#n`, `n` its catalog id.

```text
auth::identities.unique(provider, sub) in OAuth
auth::identities.unique#1207
```

## spec/writes/unique-name/expression-text

The name holds each expression's text. Two spellings of one
expression name the unique differently unless the spec fixes a
canonical printing of every expression.
