---
title: "Whether in_zone then to_instant gives back the instant"
pick: one
options:
  earlier:
    title: "Not in the later half of an overlap: it gives the earlier instant, and the spec says so"
  offset:
    title: "Always: a ZonedDateTime also keeps its offset"
---
Raised by the reference interpreter. A `ZonedDateTime` is a civil time
and a zone name (model.md §3.6), and `to_instant` takes the earlier
instant of an overlap (prelude.md §6.4), so the round trip loses the
later one. The spec did not say so.

## spec/zone-overlap/earlier

For an instant in the later half of an overlap, `in_zone` gives a
civil time that names two instants, and `to_instant` gives the earlier
one:

```adt fragment
-- 2026-10-25T01:30:00Z in Europe/Lisbon is 01:30 local, the second time
to_instant(in_zone(i, "Europe/Lisbon"))   -- 2026-10-25T00:30:00Z, an hour earlier
```

The value stays a civil time and a zone, and the rule is stated where
`to_instant` is.

## spec/zone-overlap/offset

A `ZonedDateTime` keeps its UTC offset too, as `java.time.ZonedDateTime`
does, and the round trip is exact. Changes the model's
representation, its order and equality, and its textual form.
