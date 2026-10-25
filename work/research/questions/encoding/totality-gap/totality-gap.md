---
title: Totality is not declarative with a stored tag
pick: any
raised-by: [encoding/tag-variants]
options:
  deferred-trigger:
    title: Deferred constraint trigger checks each parent has its variant
    assessment: {grade: costly, why: "one trigger per sum, checked at every commit"}
  write-api:
    title: Writes go only through generated functions inserting both rows
    assessment: {grade: hack, why: raw SQL bypasses it}
  live:
    title: Live with parents lacking variant rows
    excludes: [encoding/totality-gap/deferred-trigger, encoding/totality-gap/write-api]
---
With `encoding/tag-variants`, `identities` can hold a row with
`kind = 'google'` and no `google` row. The inclusion `identities ⊆ ∪
variants` is a disjunctive inclusion dependency; SQL has no declaration
for it.

## encoding/totality-gap/deferred-trigger

```sql
CREATE CONSTRAINT TRIGGER identities_total
  AFTER INSERT OR UPDATE ON identities
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION check_variant_exists();
```

plus a trigger on each variant table for `DELETE`. Enforced at commit.

## encoding/totality-gap/write-api

The compiler emits `insert_google(gid) RETURNS bigint` and friends, and
revokes direct `INSERT` on the tables from application roles.

## encoding/totality-gap/live

Queries over the sum treat a parent without a variant as absent or as an
error; no enforcement.
