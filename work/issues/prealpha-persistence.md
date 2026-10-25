---
created-at: 2026-10-25
status: wontfix
kind: feature
priority: 265
summary: "the interpreter-backed pre-alpha has no durable state, so an application loses its data on exit"
closed: 2026-10-25
depends-on:
  - prealpha-embedding-api
  - real-runtime
tags:
  - prealpha
  - engine
---

# Feature: Snapshot persistence for the pre-alpha

## Report

The pre-alpha keeps the database value in memory and writes it to the database file at each commit (write to a new file, force, rename, force the directory), and reads it at open. The format is internal and versioned, promises nothing, and is replaced when the engine lands. It goes through the runtime `Io` interface, so it needs the blocking std backend of `real-runtime`, and is tested in the simulator under crashes and faults (no acknowledged commit lost).

## 2026-10-25: wontfix

Dropped: Tony decided the first application waits until the engine is ready, so no interpreter-backed pre-alpha is built.
