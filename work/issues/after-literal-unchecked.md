---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 798
summary: "an integer literal inside an after record is not checked against the key's type"
closed: 2026-10-27
tags:
  - lang
---

# Bug: literals in after are unchecked

## Report

An integer literal inside an `after` record passes the checker without being checked against the type of the key field it stands for.

## 2026-10-27: done

A compound `after` cursor written as a literal record is checked against an `Option` of a record typed by the ordering keys, so a literal against an `I32` key is accepted when in range and refused out of range, and a literal against an `Instant` key is refused.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qlaws track.
