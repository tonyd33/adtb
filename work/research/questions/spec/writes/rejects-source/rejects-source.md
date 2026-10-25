---
title: "Which inserts may take rejects into"
pick: one
options:
  one-source:
    title: "Only an insert whose source is a single generator over a source relation or an external source; otherwise a type error"
  any-query:
    title: "Any query source"
---
Raised while applying the ruling on what `rejects into` means. Each
reject holds its record's position in the source and the record's
text, which only a source with one generator over one input defines.

## spec/writes/rejects-source/one-source

`rejects into` is allowed only on an insert whose source is a
comprehension with a single generator over a source relation or an
external source; any other source is a type error. The kernel has a
form for it, and its `load` value is `fresh`:

```text
insert r (for x in e do e) rejects into j    e a relation or external source
```

## spec/writes/rejects-source/any-query

Any query may be the source. A row computed from a join or an
aggregate has no single source record, so its position and text are
undefined.
