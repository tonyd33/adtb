---
title: NULL from an absent constructor looks like a NULL field
pick: any
raised-by: [query/plain-sql, query/unchecked]
options:
  tag-lint:
    title: Lint field reads not guarded by a tag test
  live:
    title: Live with ambiguous NULLs
    excludes: [query/null-ambiguity/tag-lint]
---
`e.email` from a LEFT JOIN is NULL both when the identity is not
`EmailPass` and when it is but `email` is NULL. `COUNT(e.email)`
silently counts one constructor.

## query/null-ambiguity/tag-lint

Reading a constructor field outside `CASE kind WHEN ...` or a narrowing
is a warning.

## query/null-ambiguity/live

Users branch on the tag by convention.
