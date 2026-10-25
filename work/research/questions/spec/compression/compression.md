---
title: "Wire compression"
pick: one
options:
  per-call-off:
    title: "Per call, off by default"
  mandatory:
    title: "Mandatory above 1 KiB"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/compression/per-call-off

Per call, off by default. Compressing attacker and secret data in one TLS frame is a compression oracle.

## spec/compression/mandatory

Mandatory above 1 KiB. CRIME-style risk.
