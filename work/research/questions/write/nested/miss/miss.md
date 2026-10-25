---
title: A path update may name a constructor the value does not have
pick: any
raised-by: [write/nested/path]
options:
  noop:
    title: Rows whose value is another constructor are unchanged
    links:
      - {rel: prior-art, to: prior/write-optics}
  error:
    title: The write fails at run time on such a row
  guarded:
    title: A path through a constructor type-checks only inside a match arm for it
  live:
    title: Live with whichever run-time behaviour falls out
    excludes: [write/nested/miss/noop, write/nested/miss/error, write/nested/miss/guarded]
---
`set { address.US.zip: "02139" }` on a customer whose address is `Intl`.
A silent skip is a wildcard arm the user did not write.

## write/nested/miss/noop

Prism semantics: `Right 5 & _Left .~ 10` is `Right 5`.

## write/nested/miss/error

As a failed cast: the statement aborts and names the row.

## write/nested/miss/guarded

```
update c in customers
set match c.address
  | US u   -> { address.US.zip: "02139" }
  | Intl i -> { }
```

Narrowing is not a primitive, so the arm is the only proof; this is
`write/nested/rebuild` with path syntax inside arms.

## write/nested/miss/live

Unspecified.
