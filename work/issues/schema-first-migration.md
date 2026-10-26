---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 705
summary: "a database cannot be created from a schema's declarations"
closed: 2026-10-25
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

## 2026-10-25: resolved

A schema's declarations apply as the first migration and the catalog
state survives crashes. The migration log rows and their hashes moved
to [`catalog-migration-log-rows.md`](catalog-migration-log-rows.md).
