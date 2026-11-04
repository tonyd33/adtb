---
created-at: 2026-11-03
status: open
kind: bug
priority: 420
summary: "a migration that renames a field and then drops it under its old name fails to resolve, and translation refuses every pinned body across it"
---

# Rename then drop in one migration fails to resolve

## Report

Found writing the translate-revert search. A written migration that renames a field and then drops it under its old name is refused with "an object this operator creates or names is gone by the migration's end"; translation then refuses every pinned body across it. Decide by evolution.md whether an operator naming an object by its old name after a rename in the same migration is valid; if valid, resolve by identity; if not, the refusal should name the stale name, and translation should not refuse bodies that never reach the field. The search writes this inverse as two migrations to avoid it.
