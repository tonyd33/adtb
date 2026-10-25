---
title: "Whether one transaction may mix ad hoc text and registered calls"
pick: one
options:
  refuse:
    title: "Refused with MixedVersions; the transaction stays open"
  allow:
    title: "Allowed"
---
Raised by the third review of the spec in docs/spec/.

## spec/mixed-versions/refuse

Refused with MixedVersions; the transaction stays open. Every transaction runs against one schema version.

## spec/mixed-versions/allow

Allowed. Two versions in one transaction.
