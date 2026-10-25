---
title: What compatibility the spec promises across engine releases
pick: some
raised-by: [substrate/new-engine]
options:
  file:
    title: Every release reads every file any earlier release wrote
    needs: [layout/format-version/read-old]
    links:
      - {rel: prior-art, to: prior/ops-sqlite-pragma}
  protocol:
    title: A stated window of client releases keeps working against a new server
  editions:
    title: A bundle states the language edition it is written in; every release compiles every edition
  catalog:
    title: System relations change only through catalog operators bundles can pin
    needs: [ops/catalog-release/catalog-smo]
---
SQLite: "Newer versions of SQLite can always read and/or write database
files created by older versions of SQLite, back to version 3.0.0." A
query language changes too: new keywords, new typing rules. Registered
bundles are rechecked when the schema changes; an engine upgrade that
changes the checker can break a bundle with no schema change.

## ops/promise/file

The SQLite promise, which the settled read-old ruling already makes.

## ops/promise/protocol

Bounded, as `ops/protocol`.

## ops/promise/editions

Rust editions: each crate states its edition; one compiler builds all
of them and they interoperate. A bundle written for edition 2027 keeps
compiling under later releases; a breaking language change is a new
edition.

```
bundle web-2.3 pins 41 (5d2e...) edition 2027
```

## ops/promise/catalog

Monitoring queries registered against system relations survive
upgrades.
