---
title: "How a revoke's target is compared with a row"
pick: one
options:
  row-form:
    title: "Resolve the revoke against the schema and print it in row form, then compare exactly"
  literal:
    title: "Compare the text as written"
---
Raised by the spec-rev30 review round 2 (R2). The row prints
`auth::identities.EmailPass (user, EmailPass.email)`; a revoke written
`identities.EmailPass (email, user)` would match nothing literally.

## spec/revoke-normal/row-form

A revoke's target is resolved against the schema and printed in row form before an
exact comparison, so a grant is revoked by the same text it was granted with.

## spec/revoke-normal/literal

The author must write the canonical row text.
