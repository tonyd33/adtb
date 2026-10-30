---
created-at: 2026-10-27
status: in-progress
kind: feature
priority: 1040
summary: "security.md section 1.3 gives the owner's caller as Service { name: r } but never names r"
branch: spec-rev30
tags:
  - spec
---

# Feature: name the schema owner role

## Report

`security.md` section 1.3 says a call with no asserted principal sees `Service { name: r }` for the owner role `r`, and nothing states what `r` is. The engine uses `"owner"`.
