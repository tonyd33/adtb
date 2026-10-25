---
title: Carrying fields by name copies fields that only share a spelling
pick: any
raised-by: [write/retag/carry]
options:
  sum-fields:
    title: Only fields declared on the sum carry; constructor fields never do
    needs-one-of: [model/sum-header, surface/shared-fields/header]
  live:
    title: Live with name-based carry
    excludes: [write/retag/name-carry/sum-fields]
---
`EmailPass.verified_at` (email confirmed) and `Google.verified_at`
(token checked) share a name and type and mean different things. A
retag silently keeps the first as the second.

## write/retag/name-carry/sum-fields

```
type Identity { user: Key users, created_at: Time }
  = EmailPass { email: Text, hash: Bytes } | Google { gid: Text }
```

Sum-level fields belong to every constructor and survive a retag by
identity; constructor fields are supplied anew.

## write/retag/name-carry/live

Name and type equality decides.
