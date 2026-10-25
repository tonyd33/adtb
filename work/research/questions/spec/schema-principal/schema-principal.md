---
title: "Where a schema's own Principal lives"
pick: one
options:
  root-first:
    title: "In the root namespace, resolvable from every module and bundle, declared only in the first migration, with log ids; prelude ids 37-40 belong to the default"
  any-module:
    title: "Any module may declare it at any version"
---
Raised by the third review of the spec in docs/spec/.

## spec/schema-principal/root-first

In the root namespace, resolvable from every module and bundle, declared only in the first migration, with log ids; prelude ids 37-40 belong to the default. caller has one type everywhere and stored principals never change encoding.

## spec/schema-principal/any-module

Any module may declare it at any version. Resolution, ids and assertion encoding are ambiguous.
