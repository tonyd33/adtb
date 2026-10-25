---
title: "Whether the surface has a form taking the one element of a set"
pick: one
options:
  none:
    title: "No surface only; a row of the snapshot is reached by a path through its key or by match on the key"
  only:
    title: "A surface form only e, faulting unless e has exactly one element"
---
The kernel has `only` (`Kind::Only` in `crates/adtb-lang/src/kernel.rs`),
which the `let` binding of an upsert uses. No surface form reaches it.

## spec/only/none

No surface form takes the element out of a set. A row is reached
through its key, by a path (`users.key(3).name`) or by `match` on the
key, each faulting `NoSuchRow` when there is none. Adding `only` later
is backward compatible: no program changes meaning.

## spec/only/only

`only e` is the one element of `e`, faulting when `e` has none or
several. It adds a partial form whose need no example has shown, and
a fault to name for a set of several elements.
