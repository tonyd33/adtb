---
title: "Which Member clauses an added unique or index has"
pick: one
raised-by: [spec/evolution/operator-type/catalog-sum]
options:
  written-scope:
    title: "Only a written in r.C scope gives Member clauses; an unscoped one has none and ranges over every constructor, later ones included"
  current-ctors:
    title: "An unscoped one has a Member clause for each constructor its relation has when it is added"
---
Raised while writing the migration log rows. The catalog said an index
or unique has a `Member` clause for each constructor it ranges over,
and did not say whether an unscoped one lists its relation's
constructors.

## spec/log-rows/unscoped-members/written-scope

An `add unique` or `add index` written without `in r.C` has no
`Member` clauses. It ranges over every constructor of its relation,
including constructors added after it. A written scope gives one
`Member` clause per constructor it names.

## spec/log-rows/unscoped-members/current-ctors

An unscoped one lists the constructors its relation has when it is
added. The rows then say the index ranges over those constructors
only, which is false once a constructor is added.
