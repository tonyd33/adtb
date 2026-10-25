---
title: "How lease expiry is decided across primary and replicas"
pick: one
options:
  logged:
    title: "Expiry computed from the last logged renewal on every node; the primary waits a further LeaseGrace (1h); an expired bundle is not live, and binds, calls and renewals are refused BundleExpired until re-registration"
  local:
    title: "Each node expires on its own clock"
---
Raised by the third review of the spec in docs/spec/.

## spec/lease-expiry/logged

Expiry computed from the last logged renewal on every node; the primary waits a further LeaseGrace (1h); an expired bundle is not live, and binds, calls and renewals are refused BundleExpired until re-registration. Primary and replica agree from replicated state; grace covers clock skew; no rebind path.

## spec/lease-expiry/local

Each node expires on its own clock. A partitioned replica keeps serving a bundle the primary has expired.
