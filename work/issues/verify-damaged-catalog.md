---
created-at: 2026-10-30
status: open
kind: feature
priority: 420
summary: a file whose catalog page is damaged fails open with Corrupt instead of opening for verify and dump
---

# Feature: Verify damaged catalog

## Request

A page the open must read gives `OpenError::Corrupt` with the mark set, because the schema comes from those rows. Test `a_file_with_a_damaged_catalog_page_opens_for_verify` is ignored. Server-task `verify` also answers `Quarantined` under commit log damage, while `Database::verify` and `query_text` run it.

## Proposal

## Open questions
