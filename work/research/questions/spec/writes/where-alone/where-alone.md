---
title: "What update x where p and delete x where p mean"
pick: one
options:
  conditional:
    title: "The write on x when p holds, and skip otherwise"
  refused:
    title: "where is allowed only with in"
---
Raised by the writes checker. The grammar of syntax §7 admits `where p`
on `update x` and `delete x` without `in`, and writes.md gave no
meaning to it.

## spec/writes/where-alone/conditional

`update x where p set e` means `if p then update x set e else skip`,
and `delete x where p` means `if p then delete x else skip`.

## spec/writes/where-alone/refused

`where` without `in` is a type error, or the grammar drops it. The
written `if` is the only form, and the grammar has one more
alternative to state.
