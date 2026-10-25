---
title: "Which versions the statements of one transaction run at"
pick: one
options:
  one-pair:
    title: "One schema version and one catalog version for every statement; a call at another is refused with MixedVersions"
  adhoc-registered:
    title: "Only mixing ad hoc text and registered calls is refused"
---
Raised by the implementability review of docs/spec/. transactions §5.2
refused only ad hoc text mixed with registered calls; registered calls
of bundles pinned to different versions, or reading different catalog
versions, could share a transaction. Extends spec/mixed-versions/refuse.

## spec/txn/one-version/one-pair

Every statement of a transaction runs at one pinned schema version and
one catalog version; a call at another is refused with
`MixedVersions`, and the transaction stays open.

## spec/txn/one-version/adhoc-registered

Only the ad hoc and registered mix is refused. Two bundles pinned at
different versions mix versions in one transaction.
