---
title: Widening only at coercion sites rejects arms that meet at a sum
pick: any
raised-by: [types/infer/hm-coerce]
options:
  annotate:
    title: The user annotates the match or the binding
  join-at-match:
    title: A match is itself a coercion site that joins its arms
    needs-one-of: [types/join/lub, types/join/lub-exact-fields]
  live:
    title: Live with arms of distinct constructor rows being a type error
    excludes: [types/infer/hm-sites/annotate, types/infer/hm-sites/join-at-match]
---
In a `let` with no annotation, `match i | Google g -> g | Apple a -> a`
unifies `Identity.Google` with `Identity.Apple` and fails.

## types/infer/hm-sites/annotate

```
let r: Identity.(Google | Apple) = match i | Google g -> g | Apple a -> a
```

## types/infer/hm-sites/join-at-match

Arms of a match are joined, as Rust's least-upper-bound coercion does
for match arms (https://doc.rust-lang.org/reference/type-coercions.html, unverified).

## types/infer/hm-sites/live

Arms return `g.id`-style values of one type, or tag result constructors.
