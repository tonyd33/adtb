---
title: "Which rows a revoke deletes when several grantors gave one grant"
pick: one
options:
  per-grantor:
    title: "Rows are keyed by grantee, privilege, target and grantor; a revoke deletes the issuer's rows, the schema owner's every grantor's, and refuses to orphan rows a deleted option supported"
  cascade:
    title: "A revoke deletes every grantor's rows and cascades to rows the deleted option supported"
---
Raised by the spec-rev30 review (G7). The owner and A (holding the option) both
give `call web-2.4` to B. The spec does not say whether the owner's revoke deletes
one row or both, whether revoking A's grant cascades to B, or who may revoke.
`grantor` in `catalog::grants` is never used.

## spec/revoke-grantor/per-grantor

A grant row is keyed by grantee, privilege, target and grantor. A revoke deletes
only the rows the issuing role granted; the schema owner's revoke deletes the rows
of every grantor. A revoke that removes a grant whose option supported another
role's rows (grantor that grantee, same privilege, a target it covers) is refused,
naming those rows. Only a row's grantor or the schema owner may revoke it.

## spec/revoke-grantor/cascade

A revoke deletes every matching row and every row derived through its option, as
SQL's `revoke ... cascade`. Silent loss of access for third parties.
