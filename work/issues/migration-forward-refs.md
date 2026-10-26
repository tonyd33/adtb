---
created-at: 2026-10-25
status: open
kind: bug
priority: 174
summary: "Source::migrate refuses a type naming Key r when an add relation comes before r's add relation in the same migration"
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
