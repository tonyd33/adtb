---
title: The type of a binder shared by an or-pattern
pick: any
raised-by: [lang/patterns/or-patterns]
options:
  same-type:
    title: Every alternative must bind the name at one type
  ctor-set:
    title: The binder has the constructor-set type of the alternatives
    needs: [lang/binder/ctor-row]
  live:
    title: Live with or-patterns only over constructors without binders
    excludes: [lang/patterns/or-binder/same-type, lang/patterns/or-binder/ctor-set]
---
In `| Google g | Apple g -> g.sub`, `g` is a `Google` row in one
alternative and an `Apple` row in the other.

## lang/patterns/or-binder/same-type

Rust: alternatives bind the same names at the same type. Under
`lang/binder/ctor-row` the rows differ, so this holds only for field
patterns, `| Google { sub } | Apple { sub } -> sub`.

## lang/patterns/or-binder/ctor-set

```
| Google g | Apple g -> g.sub     -- g : Identity.(Google | Apple)
```

The constructor-set type indexes and FKs already range over
(`index/ctor-set`); its fields are those common to its constructors at
one type.

## lang/patterns/or-binder/live

`| Google _ | Apple _ -> "oauth"` only.
