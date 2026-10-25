---
title: The time scale an Instant counts in
pick: one
raised-by: [model/time]
options:
  smeared:
    title: Every day has 86400 seconds; leap seconds are smeared or ignored
    links:
      - {rel: prior-art, to: prior/types-time}
  tai:
    title: Instants count SI seconds; converting to civil time uses a leap-second table
  utc-leap:
    title: Civil times may name a 23:59:60 second
---
`Instant` and the civil types are distinct (`model/time`). Whether
`b - a` between two instants is SI seconds or civil seconds.

## types/time-scale/smeared

Java time-scale (UTC-SLS over the last 1000 seconds of a leap day),
Postgres, Unix time, TC39 Temporal. `Instant` to civil is arithmetic.

## types/time-scale/tai

Durations between instants are exact physical time; every civil
conversion needs the table, which is versioned data.

## types/time-scale/utc-leap

`DateTime` admits second 60 on leap days.
