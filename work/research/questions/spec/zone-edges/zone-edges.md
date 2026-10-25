---
title: "What in_zone and to_instant do at the edges of the instant range"
pick: one
options:
  overflow:
    title: "They may fault ArithmeticOverflow"
  clamp:
    title: "The civil range is cut so that every conversion fits"
---
Raised by the reference interpreter. A civil time near the end of the
`Instant` range, shifted by a zone offset, can name an instant outside
it, and an instant near an end can have a civil time outside `Date`'s
range. prelude.md §6.4 listed no fault for `to_instant`.

## spec/zone-edges/overflow

`in_zone` and `to_instant` fault with `ArithmeticOverflow` when the
result leaves its type's range, as every time operation does
(prelude.md §6.2). Both are then partial.

## spec/zone-edges/clamp

`Date` and `DateTime` are narrowed by the largest zone offset at each
end, so every conversion fits. Both stay total, but the civil types
lose a day at each end, and the narrowing depends on the tz release.
