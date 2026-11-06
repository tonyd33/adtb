---
title: The persisted map's own pages can lose a write
pick: any
raised-by: [storage/readin-gate/persisted-map]
options:
  map-in-log:
    title: Check the map's pages against the retained log at open, as open-map would for them alone
  header-lsn:
    title: The checkpoint record carries each map page's LSN, so the map is checked by its parent
    links:
      - {rel: variant-of, to: layout/checksum/in-parent}
  live:
    title: Live with a stale map entry reading as a stale page or hiding one
    excludes: [storage/readin-gate/map-lost-write/map-in-log, storage/readin-gate/map-lost-write/header-lsn]
---
A map page whose write is lost holds older expected LSNs: a page newer than
its entry reads as `Stale` with found past expected (a false `Unlogged`), and
a page whose lost write the entry should catch is accepted when the entry is
equally old.

## storage/readin-gate/map-lost-write/map-in-log

The map's pages are few (8 bytes per data page), so their records are a
small fraction of the log, but finding them still scans the log unless they
are recycled last.

## storage/readin-gate/map-lost-write/header-lsn

The checkpoint record lists the map pages with their LSNs; open compares.
The checkpoint record is in the log, which recovery reads anyway. Oracle's
shadow blocks rely on ordinary redo instead (work/kb/oracle-database.md).

## storage/readin-gate/map-lost-write/live

A lost write of a map page is reported as damage of other pages, or hides
one.
