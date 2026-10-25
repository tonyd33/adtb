---
title: Whether a grant can stop at fields
pick: one
raised-by: [security/subject/relations]
options:
  whole-row:
    title: A grant covers every field of its constructors
  field-list:
    title: A grant lists the fields of each constructor it covers
    links:
      - {rel: prior-art, to: prior/security-pg-grant}
  masked:
    title: Ungranted fields read as a Masked constructor of a field-level sum
    links:
      - {rel: prior-art, to: prior/security-oracle-vpd}
---
Postgres grants per column; Oracle VPD column masking returns rows with
sensitive columns as NULL. There is no NULL here.

## security/fields/whole-row

Field privacy is a schema choice: split the private fields into a
facet relation (`sum/overlap/facets-optional`) and grant that.

```
relation contact_private : ContactPrivate key user
```

## security/fields/field-list

```
grant read identities.EmailPass { user, email } to support
```

A query projecting `pw_hash` is refused for `support`. Sum-level header
fields are listed once for the set.

## security/fields/masked

```
-- support reads email : Masked | Shown { email: Text }
from i in identities.EmailPass select match i.email | Shown s -> s.email | Masked -> "***"
```

The field's type depends on the role, so a registered query's signature
does too.
