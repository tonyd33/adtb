---
title: A new error constructor breaks every exhaustive client match
pick: any
raised-by: [project/spec/errors/closed-sum]
options:
  pinned:
    title: Error sums are versioned with the spec; bundles pin the version they match
    links:
      - {rel: variant-of, to: surface/registration/order/pinned}
  coarse:
    title: Few constructors by client action, detail as a field
  live:
    title: Accept the break; a new spec edition recompiles clients
    excludes: [project/spec/errors/new-kind/pinned, project/spec/errors/new-kind/coarse]
---
`lang/wildcard/scalars-only` forbids a catch-all arm over a sum, so a
client that matched `CommitError` stops compiling when the engine adds
a case.

## project/spec/errors/new-kind/pinned

As with schema constructors (readers first): the engine never returns
a constructor the client's pinned version lacks; it maps it to the
nearest declared one.

## project/spec/errors/new-kind/coarse

```
type CommitError = Retry { reason: RetryReason } | Rejected { report: Report } | Fatal { detail: Text }
```

Constructors stand for what the client does; reasons may grow, and
only clients that match on reasons see the break.

## project/spec/errors/new-kind/live

Error sums change only with spec editions.
