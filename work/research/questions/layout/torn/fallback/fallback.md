---
title: Atomic page writes are not available on every device
pick: any
raised-by: [layout/torn/atomic]
options:
  probe:
    title: Query the atomic unit at open and fall back to full page images when it is smaller than a page
  require:
    title: Refuse to open a database on a device without atomic page writes
  live:
    title: Live with torn pages where the device does not guarantee atomicity
    excludes: [layout/torn/fallback/probe, layout/torn/fallback/require]
---
NVMe AWUPF is per namespace format; many cloud volumes report none
(transactional.blog survey, secondary). Buffered I/O gets no guarantee.

## layout/torn/fallback/probe

`statx` reports `stx_atomic_write_unit_min/max`; the engine writes full
page images to the log only when a page exceeds the unit. Two code
paths, one tested less.

## layout/torn/fallback/require

One path; unusable on common hardware.

## layout/torn/fallback/live

Silent corruption on power loss.
