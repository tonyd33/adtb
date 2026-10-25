---
title: Which zone-carrying date-time types exist
pick: any
raised-by: [model/time]
options:
  zoned:
    title: A zoned date-time of civil time plus a named IANA zone
  offset:
    title: An offset date-time of civil time plus a fixed UTC offset
---
`model/time` gives `Instant` and civil `Date`, `Time`, `DateTime`.
Neither records the zone a user meant. Taking neither leaves zones to a
user record, `{ local: DateTime, zone: Text }`.

## types/zoned/zoned

```
type Meeting = { at: ZonedDateTime, title: Text }
m.at + days(1)       -- same wall time next day, across DST
```

java.time `ZonedDateTime`, Temporal `ZonedDateTime`. Equality compares
instant and zone; order is not defined by the instant alone.

## types/zoned/offset

java.time `OffsetDateTime`, ISO 8601 with offset. Exact, but not the
zone a rule change applies to.
