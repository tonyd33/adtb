---
title: How the tool decides that columns form a sum
pick: some
raised-by: [interop/migrate/tool]
options:
  declared:
    title: The user names the tag column and the constructors' columns
  from-checks:
    title: CHECK constraints relating a tag column to nullness are read as the sum
  profile:
    title: Null co-occurrence in the data is mined for candidate constructors
---
A table with a `type` column and ten nullable columns is a sum only
if each type's rows fill a fixed subset.

## interop/migrate/infer/declared

```
$ adt import-sql ... --sum vehicles.type=Car:wheels,Boat:sail_area
```

## interop/migrate/infer/from-checks

`CHECK ((type = 'car') = (wheels IS NOT NULL))` states the mapping
outright; schemas written with care have them, most do not
(unverified).

## interop/migrate/infer/profile

Group rows by tag value and by which columns are non-null; each tag
whose rows share one null pattern is a candidate constructor. A
proposal, never applied without confirmation.
