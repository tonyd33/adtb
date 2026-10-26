---
title: "Whether an example block may rely on what another block declares"
pick: one
options:
  in-block:
    title: "Each block declares or restates every temp relation and type it uses beyond the running schema"
  chained:
    title: "A block may continue an earlier one, and the corpus checks them in sequence"
---
Raised by the file scan and temp relation work. `interop.md` §4.2's
load names `identity_rejects`, which no block before it declares, and
§4.3's duplicate removal reads `identity_stage`, which the block before
it declares as a temp relation of its own text.

## spec/examples/self-contained/in-block

Each block states what it needs beyond the running schema of its part:
§4.2's load declares `temp relation identity_rejects : RejectedRow key
(load, line)`, and §4.3's second text restates `IdentityStage` and
`identity_stage`. Every block checks on its own, and the corpus test
needs no table of continuations.

## spec/examples/self-contained/chained

The corpus test keeps a table of blocks that continue earlier ones and
checks their items in sequence. The spec text stays as it is, and the
dependency lives only in the harness.
