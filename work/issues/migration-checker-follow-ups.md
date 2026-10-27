---
created-at: 2026-10-26
status: open
kind: feature
priority: 560
summary: "structural operators ignore log grants, and a recheck error in rewritten text not copied from the migration is reported at the block"
tags:
  - lang
---

# Feature: migration checker follow-ups

## Report

- Log grants are stored in `Source::grants` by catalog id, but structural operators do not carry them: a merge should be refused until it restates the grants on the constructors it merges, and dropping a granted object should drop or refuse its grants.
- `Refusal::recheck_from` locates a recheck error only in text an operator copied from the migration. An error in text an operator rewrote (a renamed field, a merged match) is still reported at the migration block.
