# Pony

kind: language
home: https://tutorial.ponylang.io/

## Arithmetic

- Default integer operators wrap on overflow and define x / 0 = 0 and x % 0 = 0; partial operators (+?, /? ...) raise an error on overflow and division by zero; unsafe operators (+~ ...) leave both undefined; addc, divc ... return (result, overflowed: Bool). [V 2026-10-25] https://tutorial.ponylang.io/expressions/arithmetic.html
