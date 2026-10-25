---
title: Unicode normalization and collation
---
UAX #15: NFC, NFD, NFKC, NFKD; a normalized string stays normalized in future versions except for then-unassigned characters (https://unicode.org/reports/tr15/). UTS #10: collation sort keys compare by binary order; DUCET weights change between versions; levels give accent- and case-insensitive comparison (https://unicode.org/reports/tr10/). Swift String equality is canonical equivalence of grapheme clusters. Postgres records a collation's provider version and warns on mismatch since indexes may be corrupt; builtin pg_c_utf8 sorts by code point (https://www.postgresql.org/docs/current/collation.html).
