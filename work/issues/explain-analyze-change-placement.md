---
created-at: 2026-11-05
status: open
kind: bug
priority: 560
summary: "catalog.md 7.2 shows change and constraints as members of explain analyze's result row; the engine's row is { document, text } with them inside the document"
---

# Explain analyze puts change and constraints in the document, not the row

## Report

## Report

catalog.md §7.2 (near line 1599) shows an analyzed write's result as `{ text: ..., document: ..., change: { identities.Google: -1 +1 }, constraints: checked }`. The engine returns `{ document, text }` and carries `change` and `constraints` inside the document and the text. A client reading the row's `change` member as the spec shows finds none.

Found in script batch 6, 2026-11-05. The explain scripts compare `change` and `constraints` where the engine puts them.

## Proposal

Decide which the contract is: the row members as the spec shows, or members of the document (then §7.4 lists them and §7.2's example changes). Fix the other side and the explain scripts with it.
