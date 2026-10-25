---
title: Whether a relation-level sum has one key across its constructors
pick: one
raised-by: [sum/relation]
options:
  one-key:
    title: The sum declares a key that every constructor carries
    links:
      - {rel: gains, to: fk/to-sum}
  tagged-key:
    title: Each constructor declares its own key and the sum key is constructor plus that key
---
`fk/to-sum` needs a target index over the whole sum. Either that index
is on fields every constructor shares, or a reference to the sum carries
the tag.

## model/one-key

```
type Identity = EmailPass { email: Email } | Google { gid: Text }
relation identities : Identity
  key id
  unique (gid) in Google
```

The key index is a ctor-set index over all constructors
(`index/ctor-set`). A retag keeps the key, so referrers keep their value.

## model/tagged-key

```
type Identity = EmailPass { email: Email } | Google { gid: Text }
relation identities : Identity
  key (email) in EmailPass
  key (gid) in Google
-- a reference to identities holds (ctor, key): EmailPass "a@b" or Google "123"
```

Natural keys need not share a type. The sum key is a value-level sum of
constructor keys.
