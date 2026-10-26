---
created-at: 2026-10-25
status: open
kind: feature
priority: 712
summary: "canonical_term does not check that a bare lowercase name it prints is a local or prelude name"
tags: [lang]
---

# Feature: check lowercase names in canonical terms

## Report

`canonical_term` debug-checks that every uppercase and qualified name resolved, but not lowercase ones: telling a local from an unresolved relation or function needs the checker to record binders and field labels, at about 30 binding sites. Record them and extend the check.
