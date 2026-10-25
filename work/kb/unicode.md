# Unicode

kind: format
home: https://www.unicode.org/

## Normalization (UAX #15)

- Four forms: NFD canonical decomposition; NFC canonical decomposition then canonical composition; NFKD compatibility decomposition; NFKC compatibility decomposition then canonical composition. [V 2026-10-25] https://unicode.org/reports/tr15/
- Stability: a normalized string stays normalized under all future Unicode versions, except text holding characters unassigned in the normalizing implementation's version. [V 2026-10-25] same

## Collation (UTS #10, UCA)

- A sort key is an array of weights whose binary comparison gives the same result as the collation of the original strings. [V 2026-10-25] https://unicode.org/reports/tr10/
- DUCET weights may change between UCA versions, so collation results are not stable across versions. [V 2026-10-25] same
- Levels: primary (base letters), secondary (accents), tertiary (case); comparing at fewer levels gives accent- or case-insensitive comparison. A deterministic comparison judges equal only canonically equivalent strings, by an identical-level tie-break. [V 2026-10-25] same
