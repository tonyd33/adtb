---
title: EdgeQL paths and cardinality
---
`User.identities.provider` traverses links and yields a distinct set of
objects. Every expression carries a static cardinality (Empty, One,
AtMostOne, AtLeastOne, Many); a missing value is the empty set, with
`??` to coalesce. Backlinks are `.<link[is Type]`. Shapes give nested
results. https://docs.geldata.com/reference/reference/edgeql/paths,
https://docs.geldata.com/reference/reference/edgeql/cardinality
