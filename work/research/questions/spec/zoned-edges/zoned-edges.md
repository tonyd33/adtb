---
title: "What zoned does at the edges of the instant range"
pick: one
options:
  overflow:
    title: "It may fault ArithmeticOverflow, as in_zone and to_instant do"
  total:
    title: "It is total; only to_instant of its result faults"
---
Raised by the report on `spec/zone-edges`, which ruled only `in_zone`
and `to_instant`. `zoned(dt, zone, o)` names an instant, and a civil
time near an end of the range, shifted by the zone's offset, can name
one outside it.

## spec/zoned-edges/overflow

`zoned` faults with `ArithmeticOverflow` when the instant its result
names leaves the range of `Instant`. Every zone function then follows
one rule at the edges.

## spec/zoned-edges/total

`zoned` builds the value without resolving its instant. The fault moves
to a later `to_instant`, far from the call that made the value, and a
stored `ZonedDateTime` could name no instant.
