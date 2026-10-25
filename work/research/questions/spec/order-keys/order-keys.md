---
title: "How compound order keys are written"
pick: one
options:
  list:
    title: "A list of keys with directions: by (a desc, b)"
  record:
    title: "An anonymous record ordered by field order"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/order-keys/list

A list of keys with directions: by (a desc, b). Anonymous records stay unordered, and field order does not affect type equality.

## spec/order-keys/record

An anonymous record ordered by field order. Equal types could order differently.
