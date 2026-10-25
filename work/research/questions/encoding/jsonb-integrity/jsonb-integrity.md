---
title: jsonb payload loses integrity on inner fields
pick: any
raised-by: [encoding/jsonb]
options:
  schema-check:
    title: CHECK against a JSON schema per constructor
  generated-cols:
    title: Promote keyed and referenced fields to generated columns
  live:
    title: Live with unchecked payloads
    excludes: [encoding/jsonb-integrity/schema-check, encoding/jsonb-integrity/generated-cols]
---
No NOT NULL, FK or type on fields inside `data`; no planner statistics
on them.

## encoding/jsonb-integrity/schema-check

`CHECK (jsonb_matches_schema(schema_for(kind), data))`, e.g. with the
pg_jsonschema extension (unverified API).

## encoding/jsonb-integrity/generated-cols

```sql
gid text GENERATED ALWAYS AS (data->>'gid') STORED
```

then `UNIQUE` and FKs on the generated column. Postgres allows FKs from
stored generated columns (unverified).

## encoding/jsonb-integrity/live

Validate in the application only.
