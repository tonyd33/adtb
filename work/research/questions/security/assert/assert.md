---
title: Where the caller's identity comes from
pick: some
options:
  connection:
    title: The role the connection authenticated as
  asserted:
    title: An authenticated application role asserts an end-user principal per transaction
    links:
      - {rel: prior-art, to: prior/security-sqlserver-rls}
  token:
    title: The engine verifies a signed end-user token and builds the principal from its claims
    links:
      - {rel: prior-art, to: prior/security-supabase}
      - {rel: variant-of, to: net/authn/token, for: per-transaction end user rather than per connection}
---
Embedded callers share the process; server callers authenticate (the
handshake is a network question). End users usually sit behind an
application server that holds one database role.

## security/assert/connection

One database role per end user, or the application's role only.

## security/assert/asserted

```
transact as User { id: k }
  return my_invoices()
```

SQL Server's `SESSION_CONTEXT` with `@read_only`: the application sets
the user id once per connection; the engine trusts the application
role to tell the truth. The right to assert is itself granted.

## security/assert/token

```
transact with token "eyJ..."
  return my_invoices()
```

Supabase: requests carry a JWT; `auth.jwt()` exposes claims. The
engine holds the issuer's verification key; the application cannot
forge a user.
