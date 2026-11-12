---
created-at: 2026-11-12
status: open
kind: bug
priority: 300
summary: "split and merge of a constructor of a value type leave dotted index elements that name it, so the migration is refused with an unrelated checker error"
tags:
  - class-identity-by-spelling
---

# Value-level split and merge do not rewrite dotted elements

## Report

With `type P = X { g: Text } | Y { .. }` used as field `p` of a relation and `index (p.X.g) in S`, `split constructor P.X` and `merge constructor P.(X | Y)` are refused with checker errors that name neither the clause nor the operator ("`.C` selects constructors of a set; a single value is examined by `match`", "`X` is not a constructor of P"). Without the dotted clause both apply and keep ids. `rename constructor P.X` does rewrite the element to `p.X3.g` and keeps its id. Likely pre-existing; found in the structure-lang-identity step 3b review.

Direction: value-level structural operators apply the one re-scope rule to element paths: a clause whose path names a replaced constructor is rewritten to the rows it covered (or refused naming the clause), the rest kept. Test: both operators with the dotted clause, expected texts and ids from catalog.md §1.2.

Defect class: Catalog rows and identity by position or spelling.
