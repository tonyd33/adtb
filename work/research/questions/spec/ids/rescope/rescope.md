---
title: "Which indexes a constructor-replacing operator re-scopes"
pick: one
raised-by: [spec/ids/rescoped-index/new]
options:
  named:
    title: "Split, merge, nest and unnest re-scope an index whose clause names a replaced constructor; one with no in keeps its id"
  every:
    title: "Every index of the relation over a replaced constructor, an index with no in included"
---
Raised by the catalog id work. A re-scoped index is a new index with a
new id, and nothing said whether `unnest` re-scopes, nor whether an
index over every constructor, written with no `in`, counts.

## spec/ids/rescope/named

`split`, `merge`, `nest` and `unnest` re-scope exactly the uniques and
indexes whose clause names a constructor the operator replaces. An
index with no `in` ranges over every constructor before and after, and
keeps its id.

## spec/ids/rescope/every

Every index whose constructor set holds a replaced constructor is
re-scoped, an index with no `in` included, so its id changes though
its clause does not.
