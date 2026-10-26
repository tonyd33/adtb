---
created-at: 2026-10-25
status: open
kind: feature
priority: 710
summary: "the engine catalog reads adtb-lang syntax trees and re-lexes source"
---

# Feature: the engine catalog consumes kernel forms only

## Report

kernel.md states the engine and the reference share only typed kernel terms. `catalog/rows.rs` uses `ast::Decl`/`RelationClause` (`relation_decl`), `lex` (`generate_text`) and `ExprKind`/`QName` (`indexed_fields`); `catalog/mod.rs` uses `OperatorKind` (`declares`) and `parse_migration` in `fold`/`migrate`. `schema::IndexDef.fields` is a `Vec<ast::Expr>`. Expose what the catalog needs as stated adtb-lang functions over resolved forms, then drop the imports. Overlaps migration-path-indexes.
