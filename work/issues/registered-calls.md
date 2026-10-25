---
created-at: 2026-10-25
status: open
kind: feature
priority: 815
summary: "no registered queries or transactions, and no principals or definer rights in process"
depends-on:
  - in-process-api
tags:
  - engine
---

# Feature: registered queries and transactions with definer rights

## Report

Implement `docs/spec/applications.md` §1, §2 and §7 and
`docs/spec/security.md` §1 and §4: registered queries and transactions
run with definer rights, principals in process, the ad hoc role.

Prerequisite: `in-process-api.md`.
