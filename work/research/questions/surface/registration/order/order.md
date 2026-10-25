---
title: A new bundle mentions a constructor the live schema lacks
pick: any
raised-by: [[evolution/old-readers/readers-first, evolution/breaks/invisible/registered]]
options:
  pinned:
    title: Each bundle is checked against the version it names; declared constructors stay write-disabled
    links:
      - {rel: prior-art, to: prior/surface-pgroll}
  prospective:
    title: Bundles register against a pending schema applied later
  live:
    title: Live with registering after the migration
    excludes: [surface/registration/order/pinned, surface/registration/order/prospective]
---
Readers must have the `Apple` arm before writers emit `Apple`. Their
queries name `Apple`, so they cannot check against a schema without
it; once `Apple` is declared, old bundles' matches are no longer
exhaustive.

## surface/registration/order/pinned

```
-- 0009: declared, write-disabled
add constructor Identity.Apple { sub: Text }

-- bundles/web.adb: has the Apple arm
bundle web-2.4 pins 9 (c7d2...)

-- 0010: once no bundle pinning 8 is active
enable identities.Apple
```

Old bundles stay exhaustive at their version because no `Apple` row
can exist while they are active. pgroll and InVerDa serve two versions
the same way.

## surface/registration/order/prospective

`bundle web-2.4 pins pending 9`; the migration applies only
once every active bundle checks against it.

## surface/registration/order/live

New clients deploy after the migration; old clients see `Apple` rows
or the migration is refused.
