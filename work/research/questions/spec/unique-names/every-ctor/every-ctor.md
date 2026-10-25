---
title: "How a unique whose in clause names every constructor is named"
pick: one
options:
  unscoped:
    title: "As the unique with no in: the same scope, one canonical name"
  as-written:
    title: "With its in clause, as any scoped unique"
---
Raised while ruling `spec/unique-names/unscoped`. `unique (f) in
(EmailPass | Google | Apple)` and `unique (f)` range over the same rows
of `identities`, and the naming rule gave them different names.

## spec/unique-names/every-ctor/unscoped

A unique whose `in` names every constructor of the sum, by a list or a
named set, has the scope of one with no `in` and its name:

```text
auth::identities.unique(user)
```

A constraint's name is canonical: one scope has one name however the
declaration writes it.

## spec/unique-names/every-ctor/as-written

The name keeps the `in` clause. Two spellings of one constraint then
get two names, and the least-name rule for violations depends on the
spelling.
