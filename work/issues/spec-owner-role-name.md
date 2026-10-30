---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 836
summary: "security.md section 1.3 gives the owner's caller as Service { name: r } but never names r"
closed: 2026-10-29
tags:
  - spec
---

# Feature: name the schema owner role

## Report

`security.md` section 1.3 says a call with no asserted principal sees `Service { name: r }` for the owner role `r`, and nothing states what `r` is. The engine uses `"owner"`.

## 2026-10-29: closed

Stated by spec revision 30, landed at a478f4ba. Review: work/briefs/review-spec-rev30.md, four rounds. Branch check 2828 passed, 7 skipped, campaign 47. Code gaps: grant-spec-rev30-code.

## 2026-10-29: resolved
