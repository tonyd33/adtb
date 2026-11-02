---
title: A pinned upsert's conflict branch meets a row of a constructor its pin lacks
pick: one
raised-by: [evolution/old-readers/readers-first]
options:
  old-blocks:
    title: A conflict branch using old reads the whole relation, so it blocks enable
  typed-refusal:
    title: Enable passes and the call is refused with a typed reason
    links:
      - {rel: rivals, to: evolution/old-readers/readers-first}
---
A bundle pinned before `Link` runs this, and the key already belongs to a
`Link` row:

```
insert notes.Plain { id: notes.key(1), body: "x" }
  unless conflict on key else update old set Plain { old with body: old.body + "!" }
```

`old` has a type at the pin that cannot describe the row. The conflict probe
itself needs no type: it compares keys. Only `old` does.

## evolution/old-readers/upsert-conflict/old-blocks

A conflict branch that uses `old`, by a field, a match, a `with` copy or
whole, counts as reading the whole relation variable, and blocks `enable r.C`
and registers as stale past an enabled `C`, as a scan does. A branch that does
not use `old` does not block. A bundle declaring `hide new constructors` is
exempt as with a scan, since a hidden row fails by key instead of binding
`old`. A `with` copy blocks although it reads no field, since `old` cannot be
typed for the row, so this is stricter than the read rule for fields.

## evolution/old-readers/upsert-conflict/typed-refusal

`enable` passes and the call fails with a named refusal instead of
`Internal`. Cheaper, but a schema change then breaks a live call, which the
spec forbids today, so the spec would have to change.
