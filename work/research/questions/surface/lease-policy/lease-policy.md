---
title: A lease expiring retires a client that is merely offline
pick: any
raised-by: [surface/reader-tracking/dormant/lease]
options:
  translate-stale:
    title: An expired bundle no longer blocks changes but keeps running through translation
    needs: [surface/version-serving/translate]
  explicit-only:
    title: Leases inform; only an explicit retire removes a bundle
  live:
    title: Live with offline clients breaking after expiry
    excludes: [surface/lease-policy/translate-stale, surface/lease-policy/explicit-only]
---
A phone offline for longer than its bundle's lease comes back to a
retired bundle and fails, and it cannot re-register because clients
send registered names, not query text.

## surface/lease-policy/translate-stale

Expiry only means the bundle stops holding back schema changes. When it
reconnects it is served at its pinned version through operator
inverses, or told precisely which constructor it can no longer see.

## surface/lease-policy/explicit-only

Leases are reported to the operator, who retires bundles by hand.

## surface/lease-policy/live

Expiry retires.
