---
title: One schema change applied to many tenant files
pick: any
raised-by: [security/tenancy/file-each, security/tenancy/relation-each]
options:
  fleet:
    title: A tool applies the operator log to each tenant and pins bundles per tenant
  live:
    title: Live with tenants on different schema versions for a while
    excludes: [security/tenancy/fleet/fleet]
---
The operator log is per database (`surface/version/counter`).

## security/tenancy/fleet/fleet

```
$ adt migrate --fleet tenants/ 0010
acme ok  beta ok  gamma refused: bundle web-2.2 live
```

## security/tenancy/fleet/live

Clients connect per tenant with the bundle that tenant's version admits.
