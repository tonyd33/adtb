---
title: A feed may carry rows of a constructor its subscriber cannot type
pick: any
raised-by: [net/feed/changes, net/feed/live]
options:
  bundle:
    title: A subscription belongs to a bundle and is encoded at its pinned version
    needs: [surface/registration/unit/bundle]
  live:
    title: Live with feeds outside readers-first
    excludes: [net/feed/version/bundle]
---
A change feed of `Identity` started before `Apple` existed would, after
`enable identities.Apple`, carry rows the consumer has no arm for.

## net/feed/version/bundle

The subscriber counts as a reader of its bundle, so `enable` waits for
it like any other (`surface/reader-tracking/bundles`).

## net/feed/version/live

Feed consumers handle unknown constructors themselves.
