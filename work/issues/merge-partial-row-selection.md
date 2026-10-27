---
created-at: 2026-10-26
status: open
kind: bug
priority: 823
summary: "a merge keeps a row selection naming only some of the merged constructors, so the recheck refuses it with the wrong message"
tags:
  - quality
  - lang
---

# A merge over a row selection naming some of the merged constructors is refused by the recheck, not the merge

## Report

With `whole` set (a merge), `reselect_rows` (`crates/adtb-lang/src/evolve/ctor.rs:510-514`, `if let Some(Some(out))`) keeps a selection that names only some of the merged constructors. `reselect` refuses the same case for key types and named sets with `the merge is refused: ... names some but not all of its constructors`.

A merge of `Place.(Region | Settlement | Site)` in a schema holding `fn region_name (l: Place.Region)` or `from l in places.Region` passes the operator and is refused only by the recheck, with ``Region` is neither a constructor nor a named set of `Place``: the program is refused, but under the wrong operator's message and position. No test covers a partial row selection under a merge.

Also, `reselect_rows` calls `check::schema_refs(schema)` again (ctor.rs:469) although `merge` (ctor.rs:1316) and `split` (ctor.rs:1598) already computed `refs` from the same schema.

Fix: have `reselect_rows` return `R<Edits>` and push the same some-but-not-all diagnostic, pass `refs` in, and add a refusal test for each form.
