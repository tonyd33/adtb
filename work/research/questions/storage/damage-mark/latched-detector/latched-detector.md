---
title: A detector marking in place holds the caller's latches and maybe the serialization lock
pick: any
raised-by: [storage/damage-mark/at-detection]
options:
  callback-unlatched:
    title: The mapping walk runs only with optimistic reads and never waits on a latch
    assessment: {grade: hack, unverified: true, why: "a walk that cannot wait must give up on a latched page and leaves the mapping incomplete, which marks nothing"}
  reentrant-ser:
    title: Make the serialization lock reentrant for the marking thread
    assessment: {grade: hack, why: "removes one deadlock, not the page-latch ones"}
  defer:
    title: Defer the mapping and the mark to a latch-free point
    links:
      - {rel: same-as, to: storage/damage-mark/drain}
  live:
    title: Live with it
    excludes: [storage/damage-mark/latched-detector/callback-unlatched, storage/damage-mark/latched-detector/reentrant-ser, storage/damage-mark/latched-detector/defer]
---
`Pager::damage` is reached from `resident` while the caller may hold other
page latches (a B-link descent holds the parent during a child's read-in,
up to `LATCHES_PER_THREAD` = 5), and from a commit holding `ser` (the
commit-log status page is written under it). Mapping walks heaps and trees
and may need the very page the caller holds; `set_mark` takes `ser`;
`change_marks` latches the marks page and waits for the log.

## storage/damage-mark/latched-detector/callback-unlatched

Map with optimistic reads only, never blocking; skip `set_mark`'s `ser` when
the thread already holds it. A latched page stops the walk as `Shape`, so a
detection under a latch often maps nothing.

## storage/damage-mark/latched-detector/reentrant-ser

A reentrant `ser` fixes the commit-path case alone.

## storage/damage-mark/latched-detector/defer

Moving the mark out of the read path is what storage/damage-mark/drain and
storage/damage-mark/damage-log do; taking this resolution abandons
at-detection's premise.

## storage/damage-mark/latched-detector/live

Accept deadlocks on damage. Unsound.
