---
title: How queries are named and abstracted
pick: one
options:
  first-order:
    title: Named queries with value parameters; a view is a nullary one
    links:
      - {rel: prior-art, to: prior/typedb}
  higher-order:
    title: Functions over predicates and queries, normalised away before planning
    links:
      - {rel: prior-art, to: prior/lang-links}
      - {rel: prior-art, to: prior/lang-morel}
  views-only:
    title: Parameterless named views; parameters only on registered queries
  macros:
    title: Syntactic templates expanded before type checking
---
Queries that cannot be named and composed are rewritten by hand, and
each copy is a separate dependent for `evolution/breaks/recompile`.

## lang/abstraction/first-order

```
fn logins (u: Key users) -> Set Text =
  from i in identities where i.user == u
  select match i | EmailPass e -> e.email | Google g -> g.gid

fn active () -> Set User = from u in users where u.status == Active select u
from u in active(), l in logins(key u) select { name: u.name, login: l }
```

Inlined at use; TypeDB 3.0 functions take this form.

## lang/abstraction/higher-order

```
fn having (p: Identity -> Bool) -> Set (Key users) =
  from i in identities where p(i) select i.user
having(fn i -> match i | Google g -> g.domain == "corp" | EmailPass _ -> false)
```

T-LINQ: lambdas and query-returning functions are allowed, and
normalisation guarantees the closed, nonrecursive, effect-free result is
one flat query (Cheney, Lindley, Wadler, ICFP 2013).

## lang/abstraction/views-only

```
fn active () -> Set User = from u in users where u.status == Active select u
```

Tutorial D virtual relvars.

## lang/abstraction/macros

Malloy source `extend` and refinements reuse fragments by textual
composition (unverified as macro semantics). Errors surface in
expansions, and a dependent is the expansion.
