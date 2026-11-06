---
title: A commit must be ordered with a damage record whose object is not yet mapped
pick: one
raised-by: [storage/damage-mark/damage-log]
options:
  map-under-ser:
    title: A commit that finds unmapped damage maps it under its ser hold before it checks the marks
    assessment: {grade: costly, unverified: true, why: "a full holders walk inside the commit's critical section, only while damage is pending"}
  refuse-pending:
    title: A commit that wrote anything is refused while any damage record is unmapped
    assessment: {grade: costly, unverified: true, why: "over-refuses unrelated commits for the length of one mapping; simple and safe"}
  lsn-order:
    title: Order by LSN only; a commit below the record's LSN stands, one above is checked once mapped
    assessment: {grade: hack, unverified: true, why: "a commit above the record that commits before mapping finishes breaks the rule that a mark set first refuses it"}
  live:
    title: Live with commits that slip past unmapped damage
---
storage.md §8 orders marks and commits under `ser`: a mark set before a
commit's hold refuses the commit. With damage-log the durable fact is the
page record; the object mark follows when the mapper runs. A commit writing
the damaged heap between the two would stand though its damage was already
durable.

## storage/damage-mark/derived-order/map-under-ser

`admit_commit` sees a nonempty unmapped set and runs `holders` for each
under `ser`. Walks read pages; reading under `ser` is already allowed (the
commit writes status pages there). Cost only while damage is pending.

## storage/damage-mark/derived-order/refuse-pending

A writing commit fails `Quarantined` naming the database while any record is
unmapped. Mapping runs at the next latch-free point, normally the end of the
call that detected it, so the window is short; on a file whose mapping is
incomplete (I/O) the window lasts until a mapping completes, refusing every
writer. §8's incomplete-mapping rule marks nothing; this would change that.

## storage/damage-mark/derived-order/lsn-order

Compare the record's LSN with the commit's; insufficient for the commit
that starts after the record but before mapping.

## storage/damage-mark/derived-order/live

A commit can write a heap whose page damage is durable but unmapped. The
next read refuses it, but the write is acknowledged after the damage was
known.
