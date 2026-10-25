---
title: The precision and range of instants
pick: one
raised-by: [model/time]
options:
  micros:
    title: Microseconds in an I64
  nanos-i64:
    title: Nanoseconds in an I64
  nanos-wide:
    title: Nanoseconds with a wide range, seconds plus a nanosecond field
  declared:
    title: Each field declares its precision
---
Precision also decides whether two instants from different clients
compare equal after a round trip.

## types/time-precision/micros

Postgres: 1 microsecond, about 292,000 years either side of the epoch.

## types/time-precision/nanos-i64

Range about 1677 to 2262: 2^63 ns is about 292 years.
Birth dates are `Date`, not instants, so the range covers event times.

## types/time-precision/nanos-wide

java.time `Instant` (long seconds, int nanos), Temporal (BigInt
nanoseconds).

## types/time-precision/declared

`Instant ms`, `Instant ns`; converting to a coarser precision rounds
explicitly.
