---
title: "How a zone name is matched and kept"
pick: one
options:
  exact:
    title: "Matched by exact spelling and kept as written"
  case-insensitive:
    title: "Matched ignoring case and kept in the tz database's spelling"
  canonical:
    title: "Matched exactly, and a link name kept as its canonical zone"
---
Raised by the reference interpreter's zone functions. prelude.md §6.4
said a zone name is an IANA name and faults `UnknownZone` otherwise,
without saying how names compare or what a `ZonedDateTime` keeps.

## spec/zone-names/exact

A zone name matches a name of the tz database release in force exactly,
code point for code point, and a `ZonedDateTime` keeps the name as
written, links included:

```adt fragment
in_zone(i, "Europe/Lisbon")   -- a ZonedDateTime in Europe/Lisbon
in_zone(i, "europe/lisbon")   -- faults UnknownZone
```

Two values naming one zone by a link and by its target are distinct
values that name the same instant.

## spec/zone-names/case-insensitive

Names match ignoring ASCII case, and the value keeps the database's
spelling. Forgiving, but the stored name differs from the one given,
and equality of zoned values depends on a folding rule.

## spec/zone-names/canonical

A link name (`US/Pacific`) is stored as its target
(`America/Los_Angeles`). Equal zones become equal values, but a tz
release that moves a link changes stored values, and the text a client
wrote does not come back.
