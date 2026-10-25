---
title: "How revert names objects and what it refuses"
pick: one
options:
  by-id:
    title: "Inverses computed by object id before the operator, resolved to current names at apply; fill clauses; dependants by object id"
  by-name:
    title: "Inverses computed by name against the schema before the revert"
  by-id-no-fill:
    title: "By object id, and a revert whose inverse needs a fill is refused"
---
Raised by the implementability review of the spec in docs/spec/. The
spec computed a revert's inverse against the schema before the revert,
but a later rename or a dropped field's lost values make that inverse
name the wrong object or have no fill. "Depends on an object n created"
was not defined.

## spec/evolution/revert-identity/by-id

Inverses are computed by object id against the schema before the
operator, and resolved to current names when the revert applies.
`revert n` takes a `fill` clause for each inverse that needs one, and
is refused without it. A later migration depends on `n` when one of
its operators names an object id that `n` created; a revert of `n` is
refused while a dependant is applied:

```adt
-- 0015 drop field Identity.EmailPass.hash; 0016 rename type Identity to Login
revert 0015
  fill Identity.EmailPass.hash = b""
-- adds Login.EmailPass.hash, the object 0015 dropped under its current name
```

## spec/evolution/revert-identity/by-name

The inverse is computed from names against the schema just before the
revert. After the rename above, `revert 0015` names `Identity`, which
no longer exists, and a name freed and reused names a different
object.

## spec/evolution/revert-identity/by-id-no-fill

Names resolve by id, but a revert whose inverse needs values it cannot
compute is refused. A migration with a `drop field` could never be
reverted, though the author can state the fill.
