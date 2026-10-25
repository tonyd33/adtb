---
title: "What the version part of a collation name names"
pick: one
options:
  cldr:
    title: "The CLDR release of the tailoring data: de@cldr-49"
  icu:
    title: "The ICU release that ships the data: de@icu-74"
---
Raised by the reference interpreter's Unicode text functions. The spec
named collations by ICU release (`"de@icu-74"`, model.md §3.5,
catalog.md §3.2, prelude.md §4), but the order a collation gives is
fixed by the CLDR collation data, and an implementation need not use
ICU at all.

## spec/collation-names/cldr

The version is the CLDR release whose root collation and tailorings
the name follows:

```adt fragment
from p in people select p order by collate(p.name, "de@cldr-49")
```

Any library that implements UTS #10 over that CLDR release gives the
same keys, so the name says what is fixed and nothing about who
computes it.

## spec/collation-names/icu

The version is the ICU release, as PostgreSQL's ICU collations report
it. Familiar, but it names one library, an ICU4X or hand-written
implementation has no such number, and two ICU releases over one CLDR
release would be two names for one order.
