# LogicBlox / LogiQL

kind: language
home: https://developer.logicblox.com/content/docs4/core-reference/webhelp/constraints.html

## Constraints

- Constraints are right-arrow rules `F1 -> F2.`, meaning `!(F1, !F2)`. [S 2026-10-25] https://developer.logicblox.com/content/docs4/core-reference/webhelp/constraints.html
- Common forms: functional dependency `diastolicBPof[p] = _ -> systolicBPof[p] = _`, exclusion `reviews(p, b) -> !authors(p, b)`, mandatory/inclusive-or `Person(p) -> isMale(p) ; isFemale(p)`, uniqueness `passportNrOf[p1] = n, passportNrOf[p2] = n -> p1 = p2`. [S 2026-10-25] https://developer.logicblox.com/content/docs4/core-reference/webhelp/constraints.html

