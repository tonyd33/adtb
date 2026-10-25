# OCaml

kind: language
home: https://ocaml.org/manual/

## Extensible variants

- `type t += ...`; matching requires a default case. [V 2026-10-25] https://ocaml.org/manual/5.2/extensiblevariants.html

## Numbers

- int operations are taken modulo 2^Sys.int_size (63 on 64-bit); / and mod raise Division_by_zero. [V 2026-10-25] https://ocaml.org/manual/5.2/api/Stdlib.html
- = and < on floats follow IEEE (nan unequal to itself); compare treats nan as equal to itself and less than any other float. [V 2026-10-25] same
