---
created-at: 2026-10-25
status: open
kind: feature
priority: 705
summary: "a database cannot be created from a schema's declarations"
depends-on:
  - bootstrap-catalog
tags:
  - engine
---

# Feature: schema from declarations as the first migration

## Report

Apply a schema's declarations as one first migration
(`docs/spec/evolution.md` §3, no other operators), writing the
catalog relations of `bootstrap-catalog.md` and creating a heap per
constructor and the declared indexes.

Prerequisite: `bootstrap-catalog.md`.
