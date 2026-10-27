---
created-at: 2026-10-27
status: open
kind: bug
priority: 798
summary: "an integer literal inside an after record is not checked against the key's type"
tags:
  - lang
---

# Bug: literals in after are unchecked

## Report

An integer literal inside an `after` record passes the checker without being checked against the type of the key field it stands for.
