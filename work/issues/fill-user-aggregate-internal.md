---
created-at: 2026-11-02
status: open
kind: bug
priority: 450
summary: A user aggregate in a fill fails to apply with Internal
---

# Fill user aggregate internal

## Report

A migration whose fill calls a user aggregate fails to apply with `Internal`, `a data map fails: Unsupported(...)`. Found by the fill-code track. A legal migration should apply or be refused with a named reason. A default works.
