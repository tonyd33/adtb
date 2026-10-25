---
title: How constraints evolve with the schema
pick: any
raised-by: [evolution/smo]
options:
  carry:
    title: Each structural operator rewrites the constraints it touches
    links:
      - {rel: prior-art, to: prior/constraint-prism-icmo}
  icmo:
    title: Constraints added and dropped by their own typed operators
    links:
      - {rel: prior-art, to: prior/constraint-prism-icmo}
---
PRISM++ splits this in two: structural SMOs carry constraints through,
and ICMOs change constraints without changing structure. Operators here
have known inverses or a declared loss.

## constraint/evolve/carry

```
split constructor Identity.OAuth into Google { sub: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { sub: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }
-- check sub_present on identities.OAuth (length(sub) > 0)
--   becomes check sub_present on identities.(Google | Apple) (length(sub) > 0)
merge constructor Identity.(Google | Apple) into OAuth { provider: Google | Apple, sub: Text }
-- unique (sub) in Google, unique (sub) in Apple
--   become unique (provider, sub) in OAuth
```

Rename renames; move-field moves the field reference; nest re-scopes
by leaf constructor. A per-constructor check survives a merge as a
match on the discriminator `evolution/merge-tag/discriminator` keeps.

## constraint/evolve/icmo

```
add constraint primary_owned = ...
drop check email_length
```

Inverse pairs: add is undone by drop, drop by an add validated against the data.
