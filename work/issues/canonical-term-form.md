---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 704
summary: adtb-lang has no canonical token form and records no name resolutions
branch: adtb-mig2
---

# Feature: canonical token form of terms

## Report

Catalog operands and hashes rest on `Term` values, the canonical token
form of syntax.md §2.9: every name qualified by its module after
resolution. adtb-lang keeps no per-span resolutions, and bare
constructor names resolve by expected type, which only the checker
knows. Record each name's resolution by span in the schema builder and
the checker, and expose `canonical_term(source, operator)` covering the
literal spellings of §2.9, tested against syntax.md's examples.
