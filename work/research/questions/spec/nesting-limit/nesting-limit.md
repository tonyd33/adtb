---
title: "Nesting limit on program text"
pick: one
options:
  balanced:
    title: "64 levels; and, or and union chains count by balanced tree height"
  per-link:
    title: "64 levels; every link of every operator chain counts"
  unstated:
    title: "No stated limit; the implementation refuses at its own depth"
---
Raised by the front end fuzzer: the parser refused text nesting past 64 levels, which the spec did not state, and a generated 65-term `or` filter was refused.

## spec/nesting-limit/balanced

The spec states a nesting limit of 64 levels, counting parentheses, operands, patterns, types, writes and generators. A chain of `and`, `or` or `union` builds a balanced tree, as a list literal does, and counts the height of that tree. Other operator chains count one level per link. Long flat lists (items of a block, arms of a match, elements of a list) do not count their length.

## spec/nesting-limit/per-link

Every link of an operator chain counts one level, so a 65-term `or` is refused. Generated filters hit it.

## spec/nesting-limit/unstated

A client cannot know which text is accepted, and two implementations disagree.
