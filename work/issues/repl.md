---
created-at: 2026-10-25
status: open
kind: feature
priority: 790
summary: "there is no command-line shell to open a database file and run ad hoc text against it"
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
