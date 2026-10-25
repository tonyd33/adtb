---
title: "What bytes the version hash covers"
pick: one
options:
  value-encoding:
    title: "The wire value encoding of the resolved operator list, as values of the catalog's operator type"
  printed-text:
    title: "The migration's text as printed by the schema printer"
---
Raised by the implementability review of the spec in docs/spec/. The
hash was over the canonical printed text, so a change to the printer
(whitespace, qualification, line breaking) changed every hash and two
engine releases could disagree.

## spec/evolution/version-hash/value-encoding

The hash covers a defined byte encoding of the resolved operator list:
the wire value encoding of the operators as catalog values, of the
catalog's operator type. Printing is for review only.

```text
h(n) = BLAKE3(h(n - 1) ++ encode(operators of n))
```

## spec/evolution/version-hash/printed-text

The hash is over the printed canonical text. Every printer rule is part
of the format, and fixing a printing bug breaks every version check.
