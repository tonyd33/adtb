---
title: Postgres composite field update
---
`UPDATE mytab SET complex_col.r = (complex_col).r + 1` assigns one
field of a composite column in place.
https://www.postgresql.org/docs/current/rowtypes.html
