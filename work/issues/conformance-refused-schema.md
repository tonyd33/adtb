---
created-at: 2026-10-27
status: open
kind: feature
priority: 834
summary: "the conformance format has no directive for a schema that must be refused"
tags:
  - test
---

# Feature: a conformance directive for a refused schema

## Report

A schema the checker must refuse, such as a `Principal` that lacks `Service { name: Text }`, cannot be written as a conformance file: the format has a directive for a refused query or write, not for a refused schema. Add one, run it on both sides, and write the `Principal` near misses with it.
