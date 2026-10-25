---
title: A role granted some constructors meets a match that names all of them
pick: one
raised-by: [security/grain/ctor-set]
options:
  refuse:
    title: A query reading an ungranted constructor is refused; the role matches over its granted set
    needs: [lang/patterns/or-binder/ctor-set]
  empty-arms:
    title: The query types against the whole sum; ungranted constructors read as empty
  role-view:
    title: The role sees a schema in which the sum has only its granted constructors
  live:
    title: Live with it
---
`support` may read `identities.(EmailPass | Google)` but not `Apple`.
Exhaustiveness and authorization both look at which constructors a query
touches.

## security/grain/partial/refuse

```
from i in identities.(EmailPass | Google)
select match i | EmailPass e -> e.email | Google g -> g.gid
```

`from i in identities` is refused at registration for `support`. The
match is exhaustive over the set it reads. Adding `Apple` changes
nothing for `support`'s queries.

## security/grain/partial/empty-arms

`from i in identities select match i | ... | Apple a -> a.sub` registers;
the `Apple` arm is empty for `support`. One query text gives different
results per role, which is how RLS behaves for whole rows.

## security/grain/partial/role-view

Codegen for `support` emits `type Identity = EmailPass { ... } | Google { ... }`;
the catalog shows `support` no `Apple`. An external schema in the ANSI
three-level sense (unverified). Adding a constructor to the sum may or
may not change a role's type.

## security/grain/partial/live

The question is left to each query author.
