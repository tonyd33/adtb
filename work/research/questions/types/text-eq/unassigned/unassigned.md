---
title: NFC is stable only for characters assigned when it was computed
pick: any
raised-by: [types/text-eq/nfc]
options:
  reject-unassigned:
    title: Text containing code points unassigned in the engine's Unicode version is refused
  pin-unicode:
    title: The database records its Unicode version; upgrading renormalizes as a migration
  live:
    title: Live with stored text that a later version would normalize differently
    excludes: [types/text-eq/unassigned/reject-unassigned, types/text-eq/unassigned/pin-unicode]
---
UAX #15: a normalized string stays normalized under future versions,
except text holding characters that were unassigned for the normalizer.

## types/text-eq/unassigned/reject-unassigned

New characters become storable when the engine's Unicode tables are
upgraded.

## types/text-eq/unassigned/pin-unicode

The version is catalog state, like a collation version.

## types/text-eq/unassigned/live

Two equal-looking keys may coexist after an upgrade.
