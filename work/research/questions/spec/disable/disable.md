---
title: "How a constructor returns from enabled to declared"
pick: one
options:
  disable:
    title: "A `disable r.C` operator, enable's inverse, refused while rows of r.C exist or a live bundle can write into it"
  deprecate:
    title: "`deprecate` serves as enable's inverse"
---
Raised by the third review of the spec in docs/spec/.

## spec/disable/disable

A `disable r.C` operator, enable's inverse, refused while rows of r.C exist or a live bundle can write into it. Keeps the inverse law exact for write states; `deprecate` applies only from enabled.

## spec/disable/deprecate

`deprecate` serves as enable's inverse. Leaves the constructor deprecated, not declared, so inverse does not restore the schema.
