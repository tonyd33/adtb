---
title: What a schema change does to a registered query that no longer checks
pick: some
raised-by: [evolution/breaks/recompile]
options:
  refuse:
    title: The migration is refused while any active query fails
  retire:
    title: The migration applies and failing queries are refused at call time
  rewrite:
    title: Queries are rewritten through operators that define a rewrite
    links:
      - {rel: prior-art, to: prior/prism-bidel}
---
`rename`, `move-field` and `nest` have a mechanical query rewrite;
`add` has none, since no rewrite invents the new arm. These stack:
rewrite where defined, then refuse or retire the rest.

## surface/registration/on-change/refuse

```
$ adt migrate 0009
refused: 2 queries in bundle ios-4.2 are not exhaustive over Identity
  list_sessions: match i missing Apple
```

## surface/registration/on-change/retire

Calls to `list_sessions` from `ios-4.2` return a typed error naming the
migration.

## surface/registration/on-change/rewrite

`rename constructor Identity.Facebook to Meta` rewrites `Facebook f ->` to
`Meta f ->` in every registered query; the client's generated code
still says `Facebook`, so the wire tag must survive renames.
