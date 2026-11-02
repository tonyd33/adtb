---
created-at: 2026-11-02
status: resolved
kind: bug
priority: 398
summary: "a match eliminated on a sole constructor drops its field read, so enable passes and a pinned reader answers over a new constructor"
closed: 2026-11-02
---

# Sole constructor match hides read

## Report

`crates/adtb-lang/src/norm/mod.rs:917` eliminates a match on a sole constructor before read facts are collected, so the field access under it disappears. With `match m.tone | Ink i -> 8` in a pinned bundle, `enable Tone.Dye` passes and the reader returns 8 over a `Dye` row. The same happens through upsert `old`. Run by the pinned-reader-reduced review, not run on its parent; `case_step` was unchanged by that track.

Cause: admission reads a normal form, so normalization can erase a read the spec defines by occurrence (docs/spec/evolution.md:1227). The searching test `every_path_binding_a_stored_row_reports_the_fields_read_through_it` misses this, so its name overclaims. Fix the class: take reads from the term before such elimination, or keep the fact through it. Add the case to the search, and also fail when a line of `accesses.txt` is deleted.

## 2026-11-02: fixed

`fix: keep reads a normal form eliminates` takes field, relation and reference reads from the checked term, so normalization cannot erase them. The search takes discarding accesses through scan and upsert `old`, and fails when an access is deleted. What remains is on `read-occurrence-gaps`.

## 2026-11-02: resolved
