---
title: Source data that violates the target schema
pick: any
raised-by: [interop/migrate/tool, interop/migrate/attach]
options:
  quarantine:
    title: Violating source rows load into a typed quarantine relation for repair
    links:
      - {rel: variant-of, to: interop/load/unit/staged}
  fix-script:
    title: The tool emits a repair query per violation class, run against the source first
  live:
    title: Live with migrations failing until the source is cleaned by hand
    excludes: [interop/migrate/dirty/quarantine, interop/migrate/dirty/fix-script]
---
Constraints the old engine never enforced (cross-child uniques,
polymorphic orphans, STI rows with another type's columns set) are
enforced here at the first load.

## interop/migrate/dirty/quarantine

```
relation vehicles_rejected : VehicleRow key id
```

The load commits the clean rows only if no clean row depends on a
quarantined one; the violation report says which.

## interop/migrate/dirty/fix-script

Reported as source-side queries the user runs before the migration.

## interop/migrate/dirty/live

Repeat until the load passes.
