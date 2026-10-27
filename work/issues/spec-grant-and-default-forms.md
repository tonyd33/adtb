---
created-at: 2026-10-27
status: open
kind: feature
priority: 867
summary: "the logged Grant form holds one privilege, and AddDefault names one constructor although a default over a shared field makes several"
tags:
  - spec
---

# Feature: logged forms of grants and defaults

## Report

- `Grant { privilege, target }` in the operator log holds one privilege; a grant of several privileges is written as the whole list in canonical form in `privilege`, with `target` `None`.
- `default f = e` over a field several constructors hold makes several defaults, but `AddDefault` names one; the engine names the first in constructor order.

Ruling: a logged grant holds a list of privileges, each with its own target, and a logged default names every constructor it applies to. State both in `catalog.md`'s operator log rows and change the encodings to match.
