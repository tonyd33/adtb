---
title: A reader that never upgrades blocks the constructor forever
pick: any
raised-by: [surface/reader-tracking/bundles]
options:
  lease:
    title: Bundles expire unless renewed by a connecting client
  retire:
    title: An operator retires a bundle, whose queries are then refused
  live:
    title: Live with enable waiting on every registered bundle
    excludes: [surface/reader-tracking/dormant/lease, surface/reader-tracking/dormant/retire]
---
An installed mobile app release stays registered for years.

## surface/reader-tracking/dormant/lease

```
bundle ios-4.2 pins 8 (a91c...) lease 90d
```

## surface/reader-tracking/dormant/retire

```
-- 0012
retire bundle ios-4.2
```

Old clients get a typed "upgrade required" error instead of an
untyped row.

## surface/reader-tracking/dormant/live

Writers wait.
