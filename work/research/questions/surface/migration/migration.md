---
title: How a migration's operators are produced
pick: one
raised-by: [evolution/smo]
options:
  handwritten:
    title: The developer writes the operators
  inferred:
    title: The engine diffs two declarative schemas and proposes operators
    needs: [surface/source/declarative]
    links:
      - {rel: prior-art, to: prior/surface-gel-migrations}
      - {rel: prior-art, to: prior/surface-atlas}
---
Either way the stored migration is a list of typed operators, reviewed
before apply. The difference is who writes it.

## surface/migration/handwritten

```
-- 0008
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }
```

Rails, Liquibase. Nothing to infer, nothing ambiguous.

## surface/migration/inferred

```
$ adt migration create
did you rename Identity.Facebook to Identity.Meta? [y/n] y
wrote migrations/0008.adt:
  rename constructor Identity.Facebook to Meta
```

Gel `migration create` asks until the plan is clear; Atlas
`migrate diff` writes the planned file. The output is the same
operator file `handwritten` would produce.
