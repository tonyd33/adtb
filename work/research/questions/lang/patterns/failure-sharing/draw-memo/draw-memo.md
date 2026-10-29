---
title: The normaliser memo refuses results that draw
pick: one
raised-by: [lang/patterns/failure-sharing/memo]
options:
  live:
    title: Live with exponential normalisation of write matches with generated keys
  per-path:
    title: Record drawing results too, relying on per-path draws
  exclusive:
    title: Record a drawing result only where the sharing is between exclusive branches
---
norm.rs:591 and :787 do not memoise a result containing `Fresh` or a
volatile host call, since the volatile-hash-cons fix
(work/issues/volatile-hash-cons.md): two call sites of one inlined
function body have the same memo key, and sharing them left one `stamp`
call where the text had two. Without the memo, each path to a shared `F`
re-normalises it.

## lang/patterns/failure-sharing/draw-memo/live

22 arms take over 60 s; a write match is limited to a few arms.

## lang/patterns/failure-sharing/draw-memo/per-path

Undo the refusal. docs/design/kernel.md §4.1 already makes each path to a
`Fresh` its own occurrence, and the executor evaluated the shared term
per occurrence before the fix. Reopens the volatile-hash-cons ruling that
two call sites stay two terms.

## lang/patterns/failure-sharing/draw-memo/exclusive

The memo needs to know that two parents are exclusive branches. Nothing
in the term records that, so the lowering would have to mark it: a join
point by another name.
