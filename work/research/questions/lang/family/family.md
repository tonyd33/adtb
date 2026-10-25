---
title: The family of the read query language
pick: one
options:
  comprehension:
    title: Comprehensions with clauses in binding order
    links:
      - {rel: prior-art, to: prior/lang-linq}
      - {rel: prior-art, to: prior/lang-morel}
      - {rel: prior-art, to: prior/lang-nrc}
      - {rel: prior-art, to: prior/lang-links}
  rules:
    title: Datalog rules over constructor relations
    links:
      - {rel: prior-art, to: prior/souffle}
      - {rel: prior-art, to: prior/flix}
      - {rel: prior-art, to: prior/typedb}
  pipeline:
    title: A pipeline of relation-to-relation transforms
    links:
      - {rel: prior-art, to: prior/lang-prql}
      - {rel: prior-art, to: prior/lang-malloy}
  algebra:
    title: Relational algebra combinators
    links:
      - {rel: prior-art, to: prior/third-manifesto}
---
The surface every non-recursive read query is written in. The examples
in the space already use comprehensions (`from i in identities select
match i | ...`); this records it. Recursion is its own question, so a
comprehension surface may still define recursive relations by rules.

## lang/family/comprehension

```
from i in identities, u in users
where i.user == key u
select { name: u.name, login: match i | EmailPass e -> e.email | Google g -> g.gid }
```

Clauses in binding order, each seeing the binders before it (LINQ,
Morel). Denotes a monad comprehension, so it has the NRC equational
theory and T-LINQ normalisation behind it. `match` is an expression, so
it sits in `select`, `where` and keys alike.

## lang/family/rules

```
login(u, e.email) :- User(u), EmailPass(e), e.user = u.
login(u, g.gid)   :- User(u), Google(g),    g.user = u.
```

Constructor relations are already the primitive, so rules over them are
direct (Soufflé `$Google(..)`, TypeQL `match $p isa person`). Recursion
comes free. A match is a set of rules, one per arm.

## lang/family/pipeline

```
from Identity
| filter verified
| select match this | EmailPass e -> e.email | Google g -> g.gid
```

Each stage takes a relation and gives one (PRQL). Reads top to bottom
in evaluation order; joins are a stage naming another relation.

## lang/family/algebra

```
(Identity.EmailPass project {user, login = email})
  union (Identity.Google project {user, login = gid})
  join User
```

Tutorial D style: named operators on relation values, no binders. Arms
of a match are relation expressions; the union is written out.
