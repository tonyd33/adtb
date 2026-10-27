---
created-at: 2026-10-26
status: open
kind: bug
priority: 799
summary: "a declaration's type errors are missed mid-migration when another declaration fails to build, and the final recheck accepts it"
---

# Checks where written gap

## Report

## Report

`checks_where_written` in `crates/adtb-lang/src/evolve/mod.rs` relies on `Source::check`. When another declaration fails to build mid-migration, type checking never runs, so a field named before it is added gets through and the final recheck accepts it. Type the new declaration on its own.
