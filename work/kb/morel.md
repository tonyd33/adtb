# Morel

kind: language
home: https://github.com/julianhyde/morel

## Query language

- Standard ML plus a `from` expression with `in`, `join`, `where`, `group`, `compute`, `order`, `yield`; iterates a list and yields a list. [S 2026-10-25] https://github.com/julianhyde/morel
- `group e.deptno compute {sum over e.salary, count over ()}` with implicit labels. [S 2026-10-25] http://blog.hydromatic.net/2020/03/03/morel-basics.html
- Compiled via Apache Calcite to relational backends. [S 2026-10-25] https://github.com/julianhyde/morel
