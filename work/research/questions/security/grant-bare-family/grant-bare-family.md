---
title: Whether a call grant may name a whole family
pick: one
options:
  refuse:
    title: Refuse grant call FAMILY, which has no query and so no signature
  every-query:
    title: Expand to one family row per query the live bundles declare
---
`grant call web to app` parses. security.md 3.2 defines a family grant by query, with one recorded signature.

## security/grant-bare-family/refuse

Refuse at statement check, saying a family grant names a query. The row is per family, query and grantor, and no one signature exists for a family. A whole release is granted by `grant call web-2.4`.

## security/grant-bare-family/every-query

Expand to one row per query. A later bundle's new query stays uncovered, which is the surprise the form appears to rule out.
