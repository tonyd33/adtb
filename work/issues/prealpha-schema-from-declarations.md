---
created-at: 2026-10-25
status: wontfix
kind: feature
priority: 255
summary: "a pre-alpha database cannot be created from a schema, since migration operators are not implemented"
closed: 2026-10-25
tags:
  - prealpha
  - lang
---

# Feature: Schema from declarations for the pre-alpha

## Report

Pre-alpha databases take their schema as declarations applied as one first migration (as milestone 7 plans for the engine). Changing the schema means a new database; migrations come with milestone 8. Needs the checker to accept a declaration file as the initial schema and the catalog's fixed ids for the system relations the application can read.

## 2026-10-25: wontfix

Dropped: Tony decided the first application waits until the engine is ready, so no interpreter-backed pre-alpha is built.
