---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 174
summary: "Source::migrate refuses a type naming Key r when an add relation comes before r's add relation in the same migration"
closed: 2026-10-26
tags:
  - lang
---

# Bug: Forward references inside one migration

## Report

`Source::migrate` applies each operator to the schema the earlier ones
produced, so a type mentioning `Key identities` is refused ("no
`identities` in scope") when another `add relation` comes before `add
relation identities`.

My ruling: the declaring operators of one migration form one
simultaneous schema, and see each other whatever their order. Other
operators still apply in order. State it in `docs/spec/evolution.md`
§4.1, with a test that the running schema's declarations in any order
give the same schema.

## 2026-10-26: ruling

Conformance files constraints/02-reference-cycle and model/08-cycles fail on this, and `docs/spec/evolution.md` 4.1's own example adds a type naming `Key devices` before `add relation devices`. My ruling: within one migration, `Key r` in a type may name a relation a later operator of the same migration adds; every other name types against the operators before it. A reference cycle is declared within one migration. Record it in the design space and state it in `docs/spec/evolution.md` 3.2.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mrtsvsxs.
Points `spec/evolution/forward-refs/later-key` and `spec/evolution/split-inverse/every-product` hold the rulings.
