---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 867
summary: "the logged Grant form holds one privilege, and AddDefault names one constructor although a default over a shared field makes several"
closed: 2026-10-29
tags:
  - spec
---

# Feature: logged forms of grants and defaults

## Report

- `Grant { privilege, target }` in the operator log holds one privilege; a grant of several privileges is written as the whole list in canonical form in `privilege`, with `target` `None`.
- `default f = e` over a field several constructors hold makes several defaults, but `AddDefault` names one; the engine names the first in constructor order.

Ruling: a logged grant holds a list of privileges, each with its own target, and a logged default names every constructor it applies to. State both in `catalog.md`'s operator log rows and change the encodings to match.

## 2026-10-29: no option-only revoke

security.md 3.5 speaks of "a revoke that removes a grant option", but the grammar
has no revoke of the option alone, and a regrant without `with grant option` keeps
the option held. Removing only the option takes `revoke` then `grant` without it in
one migration (checked-registration track). Decide whether `revoke grant option for`
exists, or reword 3.5.

## 2026-10-29: closed

Stated by spec revision 30, landed at a478f4ba. Review: work/briefs/review-spec-rev30.md, four rounds. Branch check 2828 passed, 7 skipped, campaign 47. Code gaps: grant-spec-rev30-code.

## 2026-10-29: resolved
