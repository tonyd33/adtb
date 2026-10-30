---
title: Which signature a family call grant freezes
pick: one
options:
  unanimous:
    title: Freeze the one signature every live bundle declaring the query shares; refuse otherwise
  newest:
    title: Freeze the signature in the family's highest-version live bundle declaring the query
  named:
    title: The grant names the bundle whose signature it freezes
---
security.md 3.2 says a call grant naming a family covers each later bundle
"for each granted query whose signature in the later bundle equals the one
granted". A family grant names no version, so when two live bundles of the
family declare the query with different signatures, or none declares it, the
spec does not say which signature is "the one granted".

```adt
-- web-2.3: query login_key (u: Key users) -> Text
-- web-2.4: query login_key (u: Key users) -> Set Text
grant call web::login_key to app
```

## security/family-signature/unanimous

The grant resolves the query in every live bundle of the family. If none
declares it, or two declare different signatures (parameter and result types,
by type and constructor ids), the grant is refused, naming the bundles and
signatures. Otherwise it freezes that signature. A caller wanting both
versions grants each bundle's query, and grants the family once the older
bundle is released.

The maintainer recommends this. A grant never covers some live bundles of the
family and silently not others, and the refusal says what to do. The cost is
that an administrator must grant per bundle while two incompatible versions
are live.

## security/family-signature/newest

The grant freezes the signature of the query in the live bundle with the
highest version, by the digits and dots of the name. Older live bundles with a
different signature are not covered. No refusal while one bundle declares it.
The grant then covers less than its text suggests, and clients of the older
bundle fail with a missing grant that nothing reported at grant time.

## security/family-signature/named

`grant call web::login_key as of web-2.4 to app` names the bundle whose
signature is frozen. Unambiguous, but it adds a form to the grant syntax for a
case the other two options settle without one.
