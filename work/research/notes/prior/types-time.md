---
title: Instants, civil time and durations
---
java.time: Instant, LocalDate, LocalDateTime, ZonedDateTime, Duration and Period; the Java time-scale has 86400-second days, smearing leap seconds as UTC-SLS (https://docs.oracle.com/en/java/javase/21/docs/api/java.base/java/time/Instant.html). TC39 Temporal: Instant (ns), ZonedDateTime, Plain* civil types, Duration; date addition takes overflow constrain or reject, 2021-01-31 + 1 month = 2021-02-28 by default. Postgres timestamptz stores UTC microseconds without the zone; interval keeps months, days and microseconds apart.
