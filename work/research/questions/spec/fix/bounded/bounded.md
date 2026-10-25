---
title: "Where a constructing operation counts against boundedness"
pick: one
options:
  element-flow:
    title: "Only where its result can become an element of a relation of the stratum"
  any-use:
    title: "Wherever it is applied to a value derived from the stratum"
---
Raised by the boundedness analysis. semantics.md §5.10 counted every
constructing operation applied to a value of the stratum, including
one whose result is only compared.

```adt fragment
fix a = (from u in users select key u)
      union (from x in a where { k: x } != { k: me } select x)
in a
```

## spec/fix/bounded/element-flow

An operation counts only where its result can become an element of a
relation of the stratum. A use only in a condition, a comparison or an
aggregate key does not count. The `fix` above is bounded, and so is a
backlink over an `Option` reference.

## spec/fix/bounded/any-use

The text as it stood. The `fix` above is unbounded, hence partial,
though every element it adds is a key of `users`.
