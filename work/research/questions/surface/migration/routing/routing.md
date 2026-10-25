---
title: Split, merge and scripted steps carry data no diff can infer
pick: any
raised-by: [surface/migration/inferred]
options:
  holes:
    title: The inferred file has typed holes the developer fills before apply
  prompt:
    title: The engine asks for each routing expression interactively
  live:
    title: Live with inference covering only add, remove and rename
    excludes: [surface/migration/routing/holes, surface/migration/routing/prompt]
---
`OAuth { provider, sub }` becoming `Google | Apple` needs a function
from an `OAuth` row to its new constructor. A diff sees only the two
schemas.

## surface/migration/routing/holes

```
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by ?route   -- ?route : Identity.OAuth -> Identity.(Google | Apple)
```

Apply refuses a file with holes; each hole has a type the checker
reports, as typed holes in Agda or GHC.

## surface/migration/routing/prompt

Gel asks for a `USING` expression when a property type changes
(unverified).

## surface/migration/routing/live

Structural operators are hand-written after inference fails.
