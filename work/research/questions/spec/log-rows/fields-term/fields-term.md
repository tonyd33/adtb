---
title: "What a fields Term holds"
pick: one
raised-by: [spec/evolution/operator-type/catalog-sum]
options:
  bare-list:
    title: "The field list without its parentheses"
  parenthesised:
    title: "The field list with its parentheses"
---
Raised while writing the migration log rows. `fields: Term` of
`AddUnique`, `AddIndex`, `Rekey` and `Replacement` did not say whether
the parentheses are part of the term.

## spec/log-rows/fields-term/bare-list

The `fields` Term is the field list without its parentheses:
`email, name` for `add unique (email, name) in users`.

## spec/log-rows/fields-term/parenthesised

The parentheses are kept. They are syntax of the clause, not of the
list, and every row would carry them.
