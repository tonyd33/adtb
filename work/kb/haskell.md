# Haskell (language report)

kind: language
home: https://www.haskell.org/onlinereport/haskell2010/

## Numbers

- Int covers at least [-2^29, 2^29-1]; Integer is arbitrary precision. [V 2026-10-25] https://www.haskell.org/onlinereport/haskell2010/haskellch6.html
- Results of overflow or underflow on fixed-precision numeric types are undefined; an implementation may choose error (bottom), a truncated value, or a special value. [V 2026-10-25] same
- GHC Int wraps on overflow. [U]
