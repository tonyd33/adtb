---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 165
summary: "constructor stages (declared, deprecated) are not in Schema, so NotOpen is tested only through the interpreter's with_closed"
closed: 2026-10-25
tags:
  - lang
---

# Feature: constructor stages in the front-end schema

## Report

`Schema` in `adtb-lang` does not carry a constructor's stage (declared,
deprecated). `NotOpen` is tested only through the interpreter's
`with_closed`, which closes constructors by hand. The engine's bootstrap
catalog and the write path need the stage from the schema.

## 2026-10-25: resolved

Landed as `feat(lang): give schemas catalog ids and constructor stages and fix the catalog's references`. Rulings in point `spec3`, for Tony to adopt.
