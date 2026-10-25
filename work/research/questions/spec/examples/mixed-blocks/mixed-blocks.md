---
title: "How an example mixing adt code with bytes or output is fenced"
pick: one
options:
  split:
    title: "Split it into an adt block and a text block"
  text:
    title: "Keep it one text block"
  marked:
    title: "Keep one adt block and mark the output lines for the checker to skip"
---
Raised while implementing the front end against docs/spec/. Examples
in the wire, security and engine parts showed a declaration or query
and its encoding, output or plan in one `text` block, so the corpus
test never parsed their adt lines.

## spec/examples/mixed-blocks/split

A fenced block that mixes adt code with bytes or output is split into
an `adt` block and a `text` block, so every adt line is checked.
The three protocol traces in the wire part whose frames carry adt text inline stay `text`.

## spec/examples/mixed-blocks/text

Keep the mixed block as `text`. Its adt lines go unchecked and drift
from the grammar.

## spec/examples/mixed-blocks/marked

Keep one `adt` block and prefix output lines with a marker the corpus
test strips. Adds a notation of its own to the spec.
