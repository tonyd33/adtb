---
title: "Where listener peer mappings live"
pick: one
options:
  relation:
    title: "`catalog::peer_mappings` keyed by (listener, ordinal), changed by `map peer` and `unmap peer` by `schema` holders; one relation for certificate subjects and OS users; `*` matches any run"
  setting:
    title: "A Setting constructor holding the rules"
---
Raised by the third review of the spec in docs/spec/.

## spec/peer-mappings/relation

`catalog::peer_mappings` keyed by (listener, ordinal), changed by `map peer` and `unmap peer` by `schema` holders; one relation for certificate subjects and OS users; `*` matches any run. Many-valued facts are relations; Setting holds one row per constructor.

## spec/peer-mappings/setting

A Setting constructor holding the rules. Would need a collection field.
