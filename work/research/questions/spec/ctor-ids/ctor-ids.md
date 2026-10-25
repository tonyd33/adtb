---
title: "How constructor ids are assigned"
pick: one
options:
  log-order:
    title: "Deterministically in log order; shared by instantiations; heaps keyed by relation and constructor; prelude ids fixed below 1024"
  database:
    title: "Assigned by each database"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/ctor-ids/log-order

Deterministically in log order; shared by instantiations; heaps keyed by relation and constructor; prelude ids fixed below 1024. Codegen must compute the same ids the database uses, and drivers must decode prelude sums.

## spec/ctor-ids/database

Assigned by each database. Two databases from one log disagree.
