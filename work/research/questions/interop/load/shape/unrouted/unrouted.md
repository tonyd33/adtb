---
title: What happens to a row whose tag names no constructor
pick: one
raised-by: [interop/load/shape/tag-match]
options:
  wildcard:
    title: The routing match ends in a wildcard arm the user writes
  tag-parse:
    title: The tag column is parsed into the sum's tag type, then matched exhaustively
    links:
      - {rel: gains, to: lang/group/ctor-key/tag-type}
---
A match on `Text` cannot be exhaustive without a wildcard
(`lang/wildcard/scalars-only`). An unknown `"apple"` row must go
somewhere.

## interop/load/shape/unrouted/wildcard

`| _ -> reject r` or `| _ -> error`. The user picks per load; nothing
ties the text values to the sum, so adding `Apple` to the schema does
not flag the load script.

## interop/load/shape/unrouted/tag-parse

```
from r in read_csv("identities.csv", IdentityRow)
select match Identity.Tag.parse(r.kind)    -- Option Identity.Tag
  | Some { value: EmailPass } -> ... | Some { value: Google } -> ... | None -> reject r
```

The tag of a sum is a nullary sum (`lang/group/ctor-key/tag-type`), so
the inner match names every constructor and adding `Apple` breaks the
load script at recompile like any match.
