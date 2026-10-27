---
created-at: 2026-10-26
status: open
kind: bug
priority: 795
summary: "a comment after a field's delete action is taken into the action text, which comments out the fill of a drop field inverse and is lost or misparsed by alter field"
tags:
  - quality
  - lang
---

# A comment after a field's delete action corrupts the inverse of drop field and the text of alter field

## Report

`field_texts` (`crates/adtb-lang/src/evolve/revert.rs:1713-1740`) and `alter_field` (`crates/adtb-lang/src/evolve/field.rs:172-176`) find a field's delete action by scanning the declaration text from the end of its type to `item_end` (`crates/adtb-lang/src/evolve/text.rs:38`). `item_end` does not know about comments, and `field_texts` takes everything after `on delete` up to that point as the action.

With a schema field written

```adt
owner: Option (Key users) on delete clear -- keep the npc
, name: Text }
```

- `drop field Npc.owner`, then `revert` with `fill Npc.owner = None`, is refused: `the inverse `add field auth::Npc.owner: Option (Key users) on delete clear -- keep the npc fill None` does not apply: expected `fill`, found the end of the text`. The comment comments out the fill. Before d0c35cc only the type span was sliced, so this is a regression.
- `alter field Npc.owner on delete restrict` applies and silently deletes the comment.
- With a comma in the comment (`-- keep, really`) the same alter is refused with ``expected `:`, found `,` `` because `item_end` stops at the comma.
- With a comment between the type and `on delete`, `field_texts` finds no action, and the revert of an alter restores `restrict` instead of the previous action.

Fix: record the span of the delete action in `ast::FieldType` when parsing it (`crates/adtb-lang/src/parse/ty.rs`, `on_delete`), and have `field_texts` and `alter_field` use the AST spans instead of rescanning text. Add the four cases above to `evolve_tests.rs`.
