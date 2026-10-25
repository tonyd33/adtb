# TC39 Temporal

kind: system
home: https://tc39.es/proposal-temporal/docs/

## Types

- Instant (exact time, no calendar or zone), ZonedDateTime, PlainDateTime, PlainDate, PlainTime, PlainYearMonth, PlainMonthDay, Duration. [V 2026-10-25] https://tc39.es/proposal-temporal/docs/
- Instant has nanosecond precision, a count of nanoseconds since the Unix epoch. Not Baseline in browsers as of 2026-10. [V 2026-10-25] https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Temporal

## Arithmetic

- PlainDate.add takes overflow "constrain" (default, clamps: 2021-01-31 + 1 month = 2021-02-28) or "reject" (RangeError). Units added largest first: years, months, weeks as days, then smaller units as days. [V 2026-10-25] https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Temporal/PlainDate/add
- Disambiguation of DST gaps and overlaps with compatible, earlier, later, reject. [U]
