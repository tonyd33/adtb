---
created-at: 2026-11-15
status: open
kind: feature
priority: 900
summary: canonical term text of calls and qualified constructors does not parse back as source
tags:
  - lang
---

# Feature: Canonical text not reenterable

## Request

## Proposal

## Open questions

Found by structure-lang-identity step 10c-2. `canonical_term` text, now also the stored and shown text of an expression index element, does not parse back as source for calls (`app::twice ( n )`, `Instant . parse ( name )`, `app::items . key ( n )`: "expected `)`, found `(`") or qualified constructors (`app::Draft`: "no constructor `Draft` in scope, and no expected type declares it"). Other forms reparse alpha-equal.

Ruling (lead, 2026-11-15): canonical text is an identity form, not re-enterable source; it feeds the function hash and the operator Term rows, so changing it changes stored data and the migration hash chain. This ticket holds the question of a client-facing re-enterable rendering (for catalog clause text, refusals that name an element) beside the canonical form, should a client need to paste a catalog clause back.
