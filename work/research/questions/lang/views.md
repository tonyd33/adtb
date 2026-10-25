---
title: Whether views are a declared kind
pick: one
raised-by: [lang/defs-home/schema]
options:
  nullary-fn:
    title: A view is a parameterless schema function
  view-kind:
    title: Views are a separate declaration kind
---
Views are named queries in the schema; the question is whether they get their own keyword and rules.

## lang/nullary-fn

`fn logins () -> Set Login = ...`; one concept.

## lang/view-kind

`view logins = ...`, distinct from functions.
