---
title: A schema operator touches a stored view's inputs or result type
pick: some
raised-by: [derived/materialize/storage-choice, derived/materialize/refresh-kind]
options:
  rebuild:
    title: Rebuild the stored relation as a staged job, like an index build
    links:
      - {rel: gains, to: engine/ddl-online-txn/staged-job}
  translate:
    title: The operator maps the stored rows as it maps base rows
    links:
      - {rel: gains, to: evolution/smo}
---
A schema change rechecks dependents (`evolution/breaks/recompile`); a
stored view also holds data computed under the old schema.

## derived/mv-evolve/rebuild

The new definition is populated in the background and swapped in at a
staged schema state (`engine/online-states`). Always correct.

## derived/mv-evolve/translate

`rename Invoice.Open to Unpaid` changes the view's definition and its
stored constructor names, not its rows. Works for operators with an
inverse that commute with the view's query; others rebuild.
