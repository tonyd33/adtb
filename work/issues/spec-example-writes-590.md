---
created-at: 2026-10-25
status: open
kind: feature
priority: 180
summary: "writes.md:590 fails spec_examples_check"
depends-on: [check-migration-operators]
tags: [spec]
---

# Feature: writes.md:590 passes the spec example check

## Report

`docs/spec/writes.md:590` still fails `spec_examples_check`.

```adt
-- from version 12
add unique (lower(email)) in identities.EmailPass
```

```
cargo test -p adtb-lang --test spec_corpus spec_examples_check
```

Actual:

```
writes.md:590:
  writes.md:592: migration operators are not checked yet
```

Root cause: the block is a migration operator, which the checker
refuses unchecked (`check-migration-operators.md`). Nothing in the
example is wrong.
