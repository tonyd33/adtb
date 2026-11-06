---
created-at: 2026-11-05
status: open
kind: feature
priority: 480
summary: an encrypted dump names its entries data/module::rel.ctor.form in clear and its ciphertext lengths track plaintext lengths
---

# Feature: An encrypted dump shows its schema names and entry sizes

## Request

## Proposal

## Open questions

## 2026-11-05

From the S5.6 review. The log entry, which holds the schema, is sealed, but the file names under data/ (api/dump.rs:445) show every module, relation and constructor name, and each entry's length tracks its plaintext. S5.6 states this as visible in interop.md §6.1 and security.md §9.5. Direction: name entries by index with the mapping inside the sealed manifest, and pad or bucket sizes if size is judged sensitive; a design question first (a plaintext dump keeps readable names).
