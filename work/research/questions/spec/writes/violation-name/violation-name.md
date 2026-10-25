---
title: "Which violation a statement reports among several"
pick: one
options:
  canonical-least:
    title: "Every generated constraint has a canonical qualified name; the least name is reported; a fault while checking wins"
  any:
    title: "Any one of the violated constraints"
---
Raised by the implementability review of docs/spec/. "The smallest
name" ordered constraints, but keys, uniques, references and domains
had no names, so the reported constraint was undefined when one of
them was among the violated.

## spec/writes/violation-name/canonical-least

Every generated constraint has a canonical qualified name, specified
once: keys, uniques with their scope, references by relation and field
path, newtype domains by type. When a statement violates several, the
reported one is the least name. A fault raised while checking takes
precedence over violations. An upsert's clash with another write's
insert reports the unique by its name under the same rule.

## spec/writes/violation-name/any

Any violated constraint may be reported. The error then differs across
implementations and runs, which spec/violation-report/one already
refused.
