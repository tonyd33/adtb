---
title: What ad hoc text costs for naming a declaration it reads nothing of
pick: one
options:
  charged:
    title: A role without the schema catalog reads names only relations it holds a grant on, and types and constructors those relations reach; other names answer the same Denied
  free:
    title: Names are not secret; any adhoc role learns them
---
`secrets.key(1)` and `Secret { id: k, salary: 2 }` read no rows, so nothing charges them, and a wrong name answers differently.

## security/adhoc-names/charged

Holds the rule that a relation a role cannot read and one that does not exist answer the same. A role with a grant on a relation can tell the fields of its type by probing; that is stated, and a field's value stays charged.

## security/adhoc-names/free

Contradicts the fixed refusal, which then hides nothing.
