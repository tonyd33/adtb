---
title: "How the products of a split refer to the constructor being split"
pick: one
raised-by: [evolution/split]
options:
  product-set:
    title: "A product field naming the split constructor is retyped to the products' set, as every reference to it is, unless the split names the target; the routing types a carried field at its retyped type"
  must-name:
    title: "A product names its target among the products; naming the split constructor is refused"
---
Raised by the campaign manager, splitting `Location`, whose `parent`
references `Location`. The spec retypes `Key r.OAuth` to `Key
r.(Google | Apple)` and lets a routing arm carry a field whole, but did
not say how the products declare a reference to the constructor being
split, or at what type the routing carries it.

## spec/evolution/split-self-ref/product-set

A field of a product typed by a reference to the split constructor, as
`Key r.Location`, is retyped to the set of the products, as every other
reference to it is. A product may instead name its target among the
products, as `Key r.(Region | Settlement)`, resolved at the new
version. The routing is typed with every reference to the split
constructor retyped, so a field carried whole keeps its value at the
retyped type.

## spec/evolution/split-self-ref/must-name

A product field names its target among the products; a product field
typed by the split constructor is refused. The routing still types a
carried field at its retyped type.
