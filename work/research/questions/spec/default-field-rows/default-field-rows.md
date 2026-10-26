---
title: "Which field a defaults row names for a default that fills several constructors"
pick: one
options:
  per-field:
    title: "One row per field the default fills, each with the clause's text"
  first:
    title: "One row naming the first declared constructor's field"
---
Raised while writing catalog rows. `default f = e` fills field `f` of
every constructor of the relation that has it (derived.md §4.1), and
those are distinct fields with distinct field ids, but a row of
`catalog::defaults` names one `field`.

## spec/default-field-rows/per-field

Each field's default is found by its own field id, and dropping or
moving one constructor's field affects only its row.

## spec/default-field-rows/first

One row per clause, but the other constructors' fields have a default
the catalog does not show under their ids.
