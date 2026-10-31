---
title: Whether binding needs a call grant
pick: one
options:
  some-call:
    title: Refuse with Denied when the role can call nothing in the bundle, with no renewal
  no-check:
    title: Bind and refuse each call
---
`web` holds no call grant on `web-2.4` and binds it. applications.md 5 lists its refusals, with no grant among them. Binding renews the bundle's lease.

## security/bind-needs-grant/some-call

Without this, any role with a session keeps any bundle's lease alive by binding it, and so holds off expiry and any migration or revoke that waits on it. One callable the role may call suffices; `adhoc` alone does not bind. The refusal is audited as a denied call.

## security/bind-needs-grant/no-check

Simpler binding, and a lease any role can renew.
