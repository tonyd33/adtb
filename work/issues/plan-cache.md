---
created-at: 2026-10-25
status: open
kind: feature
priority: 760
summary: plans are compiled again for every statement
---

# Feature: Plan cache

## Report

Every statement compiles its plan again. My ruling: cache plans across
statements, keyed by the normalised term and the schema version. A hit
needs a canonical walk of the term with variables renumbered and
parameters remapped, about 100 lines plus tests.
