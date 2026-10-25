---
title: Rules give exhaustiveness no construct to check
pick: any
raised-by: [lang/family/rules]
options:
  match-body:
    title: A match form allowed inside a rule body
  rule-group:
    title: Rules declared as one group over a sum are checked to cover it
  live:
    title: Live with one rule per arm and no coverage check
    excludes: [lang/family/rules-cover/match-body, lang/family/rules-cover/rule-group]
---
`query/compile-time` checks a match. Written as separate rules, the arms
are independent clauses; dropping the `Google` rule is a smaller
relation, not a type error. Soufflé has no exhaustiveness check.

## lang/family/rules-cover/match-body

```
login(u, l) :- User(u), Identity(i), i.user = u,
  l = match i | EmailPass e -> e.email | Google g -> g.gid.
```

The match is an expression term, checked as anywhere else.

## lang/family/rules-cover/rule-group

```
login(u, l) by cases i in Identity:
  EmailPass e -> l = e.email.
  Google g    -> l = g.gid.
```

A declared case split over a sum; no precedent found.

## lang/family/rules-cover/live

Coverage is the author's job in rule form.
