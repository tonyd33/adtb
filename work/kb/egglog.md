# egglog

kind: language
home: https://github.com/egraphs-good/egglog

## Types

- `(datatype Expr (Num i64) (Var String) (Add Expr Expr) (Mul Expr Expr))` declares a recursive datatype. [V 2026-10-25] https://egraphs-good.github.io/egglog-tutorial/01-basics.html
- Datatype values are e-class ids; equal terms merge into one class. [U]
- Container sorts Vec, Set, Map are built in and parametric. [U]

## Datatypes

- `datatype` is sugar for `sort` plus `constructor`; datatypes are open. [V 2026-10-25] https://egraphs-good.github.io/egglog-tutorial/02-datalog.html
