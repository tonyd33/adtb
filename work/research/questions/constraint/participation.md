---
title: Whether at-least-one participation is declarable
pick: one
raised-by: [sum/relation]
options:
  none:
    title: No, only FKs from child to parent
  at-least-one:
    title: Declarable inclusion from parent into a relationship, checked at commit
---
"Every User has at least one Identity" is an inclusion dependency from
parent to child. No FK gives it; EER gives it as overlapping-total. It
is the same disjunctive inclusion that made totality hard for the
tag-plus-variants encoding.

## constraint/none

Applications ensure it.

## constraint/at-least-one

```
constraint has_identity =
  from u in users
  where count(i for i in identities where i.user == key u) == 0
  select u
```

Checked at the end of the statement that makes the change; under multiple assignment that statement may create parent and child together.
