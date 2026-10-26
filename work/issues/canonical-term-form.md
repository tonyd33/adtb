---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 704
summary: adtb-lang has no canonical token form and records no name resolutions
closed: 2026-10-25
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

## 2026-10-25: resolved

## Resolution

`canon::canonical_term(text, span, &Names)` prints an operand; `Source::names` and `Logged::names` resolve its names. Binding occurrences are qualified too (`type auth::Identity = ...`): the form is an identity for hashing and comparison, not re-parseable source. Names are resolved by rerunning the schema lookups over the built declaration; constructors resolved by expected type come from the checker.
