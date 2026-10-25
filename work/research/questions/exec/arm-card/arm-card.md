---
title: How the cardinality of a match is estimated
pick: one
raised-by: [query/union-all]
options:
  per-arm:
    title: Each arm estimated on its own heap's statistics, summed
    needs: [exec/stats/per-ctor]
  sum-scaled:
    title: Whole-sum estimate scaled by each constructor's share
    needs: [exec/stats/sum-merged]
---
Arms are disjoint, so the union's cardinality is exactly the sum of its
arms': no overlap correction as for a set union.

```
|match| = Σ_c |σ_p(identities.c)|
```

## exec/arm-card/per-arm

`|σ_{domain="corp"}(Google)| = 1.2M × 0.31`. Correlation between the
constructor and a shared field is captured, since each heap has its own
histogram.

## exec/arm-card/sum-scaled

`|σ_p(identities)| × rows(c) / rows(identities)`. One statistics object per
shared field; assumes the predicate is independent of the constructor.
