---
title: What a call grant on something absent does
pick: one
options:
  refuse:
    title: Refuse a target that does not name a registered callable, after the authority check
  allow:
    title: Keep the row, which covers nothing until the target exists
---
security.md 3.2 states a refusal only for a family grant. A bundle or query target is open.

```adt
grant call web-9.9 to a
grant call web-2.4::nosuch to a
grant call web-2.4::display_name to a  -- an imported schema function
grant call web-2.3 to a                -- web-2.3 is retired
```

## security/grant-absent/refuse

Refuse all four, naming the target. Authority is checked first, so a role that may not grant gets the same `Denied` whether or not the target exists. A superseded or expired bundle stays grantable, since it can be live again. A retired one cannot be live again. An imported function takes `grant call fn`.

A row that covers nothing is the typo the revoke rule spec/empty-revoke/refuse exists to catch.

## security/grant-absent/allow

Keep the row. A grant could precede its bundle, at the cost of a typo silently granting nothing, and of a row that later covers a bundle nobody meant.
