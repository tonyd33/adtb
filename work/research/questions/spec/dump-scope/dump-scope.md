---
title: "What a dump carries"
pick: one
options:
  all:
    title: "Roles, grants, credential verifiers, settings, audit and code bytes besides schema and rows"
  data-only:
    title: "Schema and rows only"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/dump-scope/all

Roles, grants, credential verifiers, settings, audit and code bytes besides schema and rows. A restore must keep clients working and the version hash must cover code.

## spec/dump-scope/data-only

Schema and rows only. Clients cannot authenticate after a restore.
