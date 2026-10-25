---
title: "Which write state split and unnest products start in"
pick: one
options:
  source-state:
    title: "The source's state, per relation variable"
  enabled-or-declared:
    title: "Enabled when the source is enabled, else declared"
---
Raised by the implementability review of the spec in docs/spec/. The
spec gave products of split, merge, nest and unnest a state from their
sources' states, enabled or declared, and said nothing for a split of
a deprecated constructor.

## spec/evolution/product-state/source-state

Split and unnest products take their source's lifecycle state, per
relation variable, deprecated included:

```adt
-- identities.OAuth deprecated, user_logins.OAuth enabled
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text } by match ...
-- identities.Google, identities.Apple deprecated; user_logins.Google, user_logins.Apple enabled
```

## spec/evolution/product-state/enabled-or-declared

A product is enabled when its source is enabled and declared
otherwise. A split of a deprecated constructor makes its products
declared, so `enable` reopens writes into rows the author meant to
retire, and the inverse merge does not restore the deprecated state.
