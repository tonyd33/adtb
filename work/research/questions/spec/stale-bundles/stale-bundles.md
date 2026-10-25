---
title: "Bundles that cannot be served"
pick: one
options:
  refuse:
    title: "Refuse binding an expired bundle with untranslatable queries and registering one pinned before an enabled constructor it reads"
  admit:
    title: "Admit them and refuse later migrations"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/stale-bundles/refuse

Refuse binding an expired bundle with untranslatable queries and registering one pinned before an enabled constructor it reads. Otherwise one stale bundle wedges every later migration or reads rows it cannot type.

## spec/stale-bundles/admit

Admit them and refuse later migrations. Wedges the schema.
