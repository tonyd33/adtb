---
title: How functions, views and constraint bodies are defined and changed
pick: one
raised-by: [surface/source/op-log]
options:
  in-log:
    title: define, redefine and drop are operators in the log, rechecked like any change
  modules:
    title: Definitions live in versioned module files outside the operator log
---
The schema is the fold of the operator log, but no operator creates or
edits a schema function, a view or a constraint body. `add` of a
constructor must also update every schema function that matches on the
sum.

## evolution/in-log

```
-- 0010
define fn display_name (i: Identity) -> Text = match i | ...

-- 0011
add constructor Identity.Apple { sub: Text }
redefine fn display_name (i: Identity) -> Text = match i | ... | Apple a -> a.sub
```

One migration may hold several operators; it applies as one schema
change, so the add and the redefinition land together and the recheck
sees both.

## evolution/modules

Schema functions are source files in modules, versioned with the
operator log by a manifest. The log holds data-shape operators only.
