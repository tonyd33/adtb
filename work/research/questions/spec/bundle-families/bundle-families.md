---
title: "How releases of one application relate"
pick: one
options:
  family:
    title: "Bundle names are FAMILY-VERSION; call grants and plan pins carry within a family for equal signatures or checked forms"
  unrelated:
    title: "Every bundle name is unrelated"
---
Raised by the second review of the spec in docs/spec/.

## spec/bundle-families/family

Bundle names are FAMILY-VERSION; call grants and plan pins carry within a family for equal signatures or checked forms. A new release otherwise needs fresh grants and loses its plan pins.

## spec/bundle-families/unrelated

Every bundle name is unrelated. Every release resets grants and plans.
