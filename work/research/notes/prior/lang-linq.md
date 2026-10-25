---
title: LINQ query syntax
---
`from x in xs where p select e`, clauses in binding order so each sees
the previous binders. `group x by k into g` yields groups, which are
never empty. Outer join is a group join plus `DefaultIfEmpty`. Meijer,
Beckman, Bierman, SIGMOD 2006, https://doi.org/10.1145/1142473.1142552
