---
created-at: 2026-10-27
status: open
kind: feature
priority: 836
summary: "security.md section 1.3 gives the owner's caller as Service { name: r } but never names r"
tags:
  - spec
---

# Feature: name the schema owner role

## Report

`security.md` section 1.3 says a call with no asserted principal sees `Service { name: r }` for the owner role `r`, and nothing states what `r` is. The engine uses `"owner"`.
