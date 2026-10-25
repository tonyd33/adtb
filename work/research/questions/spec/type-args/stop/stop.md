---
title: "Where a type argument list ends"
pick: one
options:
  clause-words:
    title: "A type argument list stops before a contextual clause word and before a name followed by ("
  reserve:
    title: "The clause words become reserved keywords"
  parens:
    title: "A type with arguments in a clause position is parenthesised"
---
Raised while implementing the front end against docs/spec/. Type
application is juxtaposition, so in `relation r : Box a unique (x)` or
`x: Option Int on delete clear` the grammar alone lets the type take
the following word as one more argument.

## spec/type-args/stop/clause-words

A type argument list stops before a contextual clause word of the
enclosing form (`on`, `volatile`, `unique`, `interval`, `index`,
`default`, `computed`, `fill`, `generate`) and before a name
immediately followed by `(`. The words stay contextual and usable as
names elsewhere.

```adt
relation boxes : Box Text unique (label)   -- Box Text, then a unique clause
```

## spec/type-args/stop/reserve

Reserve every clause word as a keyword, so no type argument can be one.
Takes common words such as `index` and `default` from every name space.

## spec/type-args/stop/parens

Require `(Box Text)` wherever a clause may follow. Every relation
declaration over a generic type pays parentheses.
