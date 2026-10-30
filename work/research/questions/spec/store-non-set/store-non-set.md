---
title: "Whether a view whose result is not a Set can be stored"
pick: one
options:
  refuse:
    title: "store is refused"
  admit:
    title: "store applies and the value is kept"
---
Raised by the view-panic track. `define fn total () -> I64 = count(...)` then `store total`. derived.md 2.1 admits any function with no parameters; 2.5 says a stored view is a relation.

## spec/store-non-set/refuse

A relation holds a set of rows, and `key` and `index` on a scalar mean nothing. Before the fix such a `store` panicked at apply, so no log holds one.

## spec/store-non-set/admit

The engine would need a row shape for a value that is not rows, seen by no reader, since `total()` already answers without it.
