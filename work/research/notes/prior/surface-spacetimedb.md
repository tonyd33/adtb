---
title: SpacetimeDB BSATN sums
---
Tables may hold sum types. BSATN encodes a sum value as a `u8` tag
indexing the variant list, then the variant's payload. Client bindings
are generated from the module's schema (`spacetime generate`,
unverified).
https://spacetimedb.com/docs/bsatn
