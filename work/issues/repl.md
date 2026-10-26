---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 790
summary: there is no command-line shell to open a database file and run ad hoc text against it
closed: 2026-10-25
depends-on:
  - in-process-api
  - textual-syntax-in-engine
  - real-runtime
tags:
  - engine
---

# Feature: a shell over the in-process API

## Report

A binary that opens or creates a database file, applies a schema file as the first migration, and reads ad hoc text (queries, writes, `transact`) line by line, printing results in the textual syntax. It uses only the in-process API, so it needs that API, the engine's textual syntax, and a real runtime and I/O backend behind the file.

## 2026-10-25: resolved

## 2026-10-25: resolved

`crates/adtb-shell` builds the `adtb` binary: `adtb FILE`, `adtb --create --schema SCHEMA FILE`, `adtb FILE -c TEXT`, and a script on stdin. The parser decides when a text is whole; `\begin`, `\commit`, `\rollback`, `\q`. `adtb::Error` has `Display`. Follow-ups: `shell-follow-ups`.
