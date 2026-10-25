---
title: "Syntax of schema operators"
pick: one
options:
  verb:
    title: "Verb, object, clauses"
  symbolic:
    title: "Symbolic operators"
---
One operator form per change, in numbered migration files.

## syntax/operator/verb

```
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm emails: Apple a -> { email: a.sub }
rename constructor Identity.Facebook to Meta
enable identities.Apple
```

## syntax/operator/symbolic

`Provider += Apple { sub: Text }`.
