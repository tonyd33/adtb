---
title: How a query refers to who is calling
pick: one
options:
  role-name:
    title: The connection's role name as Text
    links:
      - {rel: prior-art, to: prior/security-supabase}
  sum:
    title: A schema-declared sum Principal the engine fills from the session
---
Policies and queries compare rows with the caller. Supabase's
`auth.uid()` is null for anonymous requests, so a comparison silently
fails.

## security/principal/role-name

```
from v in invoices where v.owner_name == current_role select v
```

## security/principal/sum

```
type Principal = User { id: Key users } | Service { name: Text } | Anonymous
from v in invoices
where match caller | User u -> v.owner == u.id | Service _ -> true | Anonymous -> false
select v
```

Anonymous callers are a constructor, so every policy and query decides
them, and adding a principal kind breaks every match that must decide
it.
