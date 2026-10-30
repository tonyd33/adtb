---
title: "Whether a default covers a constructor added after it"
pick: one
options:
  named:
    title: "A default applies only to the constructors its log entry names; a new add default covers a later constructor"
  every:
    title: "default f covers every constructor that has f, later ones included"
---
Raised by the spec-rev30 review (G14). The checker applies a constructor-less
default to any constructor holding the field, while the id rules give a later
constructor no `defaults` row.

## spec/default-scope/named

A default applies only to the constructors its log entry names. A constructor
added later with field `f` does not get an existing `default f`; a new `add
default` covers it.

## spec/default-scope/every

`default f` follows the relation's constructors, which needs a defaults row and id
per later constructor created by `add constructor`.
