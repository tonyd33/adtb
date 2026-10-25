---
title: Small constructor heaps yield tasks smaller than a morsel
pick: any
raised-by: [exec/parallel/morsel]
options:
  inline:
    title: Run a single-morsel step on the calling thread
    links:
      - {rel: prior-art, to: prior/exec-umbra}
  pack:
    title: Pack several small arm pipelines into one task
  live:
    title: Live with dispatch overhead per small arm
    excludes: [exec/parallel/small-heaps/inline, exec/parallel/small-heaps/pack]
---
A sum with twelve constructors, two of them large, gives ten arm
pipelines of a few pages each. A morsel cannot span heaps without
mixing schemas, so every small arm is a task with dispatch cost and a
thread handoff.

## exec/parallel/small-heaps/inline

Umbra detects at run time that a parallel step has one morsel and runs
it without dispatch (CIDR 2020, sec. 3.3).

## exec/parallel/small-heaps/pack

One task runs `arm Apple` then `arm Facebook` over their heaps; each
keeps its own code, the task carries a list.

## exec/parallel/small-heaps/live

Per-task cost times the constructor count.
