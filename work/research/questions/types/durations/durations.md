---
title: How lengths of time are typed
pick: one
raised-by: [model/time]
options:
  split:
    title: An exact Duration and a calendar Period are distinct types
  one-interval:
    title: One interval of months, days and sub-day time
---
One month is not a fixed number of seconds; across DST one day is not
86400.

## types/durations/split

```
i + seconds(30)          -- Instant + Duration
d + months(1)            -- Date + Period
i + months(1)            -- type error: needs a zone
```

java.time `Duration` and `Period`.

## types/durations/one-interval

Postgres `interval`, kept as months, days, microseconds because their
lengths vary. `Instant + interval` interprets months and days in a
session zone.
