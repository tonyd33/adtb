---
title: Counts and aggregates over labelled data need declassification
pick: any
raised-by: [security/ifc/static, security/ifc/floating]
options:
  declared:
    title: A registered query may declare a declassification, owned and audited like a grant
  live:
    title: Live with results carrying the join of every label read
    excludes: [security/ifc/declassify/declared]
---
`count(p for p in patients where p.diagnosis == d)` carries the patients' label.

## security/ifc/declassify/declared

```
query diagnosis_counts () -> Set { diagnosis: Text, n: I64 } declassify {patient} to {clinic} = ...
```

## security/ifc/declassify/live

Most useful aggregates become unreadable to their intended readers.
