---
title: "Where a translated write's missing fields come from"
pick: one
options:
  same-ctor:
    title: "From the existing row only when it has the new value's constructor; else the current default, else the fill"
  by-key:
    title: "From the existing row with the same key, whatever its constructor; else the fill"
  fill-only:
    title: "Always the operator's fill"
---
Raised by the implementability review of the spec in docs/spec/. A
write from a bundle pinned before an `add field` does not state the
field. The spec kept it from the row with the same key, which after a
retag may be a row of another constructor, and otherwise took the
fill even when the relation has a default for later inserts.

## spec/evolution/translated-fields/same-ctor

A translated write keeps a field from the existing row only when the
existing row has the same constructor as the new value. Otherwise the
field takes the relation's current default for that constructor, else
the operator's fill. A translated insert takes the current default,
else the fill:

```adt
-- 0033 add field Identity.EmailPass.verified: Bool fill false
--      add default identities.EmailPass.verified = true
-- a bundle at 32 inserts EmailPass { ... }: verified is true
```

## spec/evolution/translated-fields/by-key

The field comes from the row of the same key. A retag from a pinned
bundle keeps a field from a row of another constructor, where a field
of the same name may mean something else.

## spec/evolution/translated-fields/fill-only

Every missing field takes the fill. An update from a pinned bundle
overwrites a field the current version set, and a translated insert
differs from the same insert at the current version.
