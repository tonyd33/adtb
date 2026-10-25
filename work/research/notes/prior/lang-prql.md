---
title: PRQL
---
Pipeline of transforms, each relation to relation: `from employees |
filter ... | group {title} (aggregate {n = count this})`. `group`
applies a sub-pipeline per partition. Compiles to SQL.
https://prql-lang.org/book/reference/stdlib/transforms/
