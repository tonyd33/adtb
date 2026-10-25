---
title: "Whether anonymous sums and records may be field types"
pick: one
options:
  named-only:
    title: "Every sum or record used as a field type is a declared type"
  inline:
    title: "Inline anonymous types as field types"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/inline-types/named-only

Every sum or record used as a field type is a declared type. Nominal typing holds without exception and evolution has a name to act on.

## spec/inline-types/inline

Inline anonymous types as field types. Structural field types escape every operator that acts on types.
