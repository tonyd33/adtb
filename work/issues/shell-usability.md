---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 870
summary: "a new user trying the adtb CLI is refused a plain schema file, reads Rust debug errors, finds a refused create's files left behind, must qualify every name and has no help"
closed: 2026-10-26
tags:
  - shell
  - docs
---

# Feature: shell usability for a new user

## Report

Trying the `adtb` CLI as a new user, in order:

1. A plain schema file is refused by `--create --schema`:

   ```adt
   module demo
   relation pets : Pet key name
   type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
   ```

   fails with `expected a schema operator, found relation`. Only migration syntax is accepted. `--schema` should take either form, replaying a schema file as the first migration.
2. Errors print as Rust debug output: `Rejected { reason: SchemaRefused { operator: "", detail: "..." } }`. `api::Error` has no `Display`.
3. A refused `--create` leaves the database file and its `-log` directory behind, so the next run opens an empty database and reports `no module demo`.
4. Ad hoc text needs qualified names (`demo::pets`) everywhere, with no way to set a default module.
5. `Int` is not a type; the prelude's integer is `I64`, and `no type Int` does not say so.

The interactive shell has no `\help` and `adtb --help` prints nothing useful.

A getting-started page (the first page of `user-manual.md`) goes with it, checked by a test so it cannot drift.

## 2026-10-26: resolved

## 2026-10-26: landed

`--schema` takes a schema file or a migration and may repeat; `api::Error` displays as plain messages; a refused `--create` removes the files it made; `no type Int` suggests `I64`; a name declared in two modules asks to be qualified; `\help` and `--help` exist. `docs/manual/getting-started.md` runs as `crates/adtb-shell/tests/manual.rs`. What a new user still trips on is `shell-new-user-follow-ups`.
