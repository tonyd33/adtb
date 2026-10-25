---
title: What a constructor no policy covers allows
pick: one
raised-by: [security/policy/predicate]
options:
  deny:
    title: Nothing, once its relation has any policy
    links:
      - {rel: prior-art, to: prior/security-pg-rls}
      - {rel: prior-art, to: prior/security-gel-policies}
  allow:
    title: Everything the role's grants allow
---
Matters when a constructor is added to a relation that has policies.

## security/default/deny

Postgres and Gel. A new `invoices.Disputed` is invisible to every
policy-bound role until a policy covers it.

## security/default/allow

A new constructor is as open as its grants.
