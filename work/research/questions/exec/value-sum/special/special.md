---
title: Special layouts for common sum shapes
pick: any
raised-by: [exec/value-sum/dense, exec/value-sum/sparse, exec/value-sum/tag-split]
options:
  option-validity:
    title: A two-constructor sum with one nullary constructor as a validity bitmap
  enum-tag:
    title: A sum of nullary constructors as its tag vector alone
  one-ctor:
    title: A one-constructor sum as its payload with no tag
---
With no NULL, optionality is a sum (`None | Some T`) and is the most
common sum there is. The general layouts spend a tag byte per row on it.

## exec/value-sum/special/option-validity

`Option T` laid out as `T` plus a 1-bit-per-row bitmap, exactly Arrow's
nullable layout. A match on it becomes a validity test; Arrow kernels
apply. Engine-internal only: the language still sees two constructors.

```
email: Option Text      -> utf8 view column + validity bitmap (bit 0 = None)
match u.email | None -> "-" | Some e -> e
```

## exec/value-sum/special/enum-tag

`Status = Draft | Sent | Paid` is a `u8` vector; filters on it are
integer compares, grouping keys are the tag.

## exec/value-sum/special/one-ctor

A sum with one constructor (a record kept as a sum for evolution) has
no tag column; adding a second constructor changes the layout.
