# Engine

This annex describes how the reference engine stores, logs, caches,
versions, indexes, executes and replicates constructor relations. It is
informative: the normative parts state guarantees over constructor
relations, and an implementation may meet them with other machinery.
"Must" here marks an invariant other parts of this engine rely on;
everything else is description. Sections run from the bottom layer up:
the file and its pages, the log and recovery, the buffer pool and I/O,
heaps and row versions, indexes, transactions, execution, replication,
and maintenance operations.

Examples use the running schema of [README.md](README.md#conventions)
at version 12, after `Apple` is added and enabled. The engine sees
it as one sum `Identity` with three constructor heaps,
`identities.EmailPass`, `identities.Google` and `identities.Apple`, and
three constructor-set indexes:

```
relation identities : Identity
  key id generate uuidv7       -- index over {EmailPass, Google, Apple}
  unique (email) in EmailPass  -- index over {EmailPass}
  unique (gid) in Google       -- index over {Google}
  index (user)                 -- local extension: index over {EmailPass, Google, Apple}
```

## 1. Scope and vocabulary

### 1.1 Shape of the engine

The engine is a database engine of its own, not a layer over another
database or a key-value store. It owns its pager, its heap and index
formats, its log and its buffer pool. Its core is a library linked into
an application; the server of [wire.md](wire.md) is a client of that
library. One process owns a database file at a time
([implementation.md §6](implementation.md#6-concurrency-model-and-non-goals)).

The engine targets an application database inside one process, with
many short transactions, and data many times larger than memory on one
node. Linux is the tuned platform; the I/O layer has portable backends
so the library embeds on other systems with slower paths (§4.3).

The catalog is a set of relations in the engine's own model, read by
the query language and written by ordinary transactions
([catalog.md](catalog.md)). Every transaction sees the catalog of its
snapshot, so a schema change is a write like any other (§7.6).

### 1.2 Terms

| Term | Meaning |
| --- | --- |
| constructor id | The number of a declared constructor of a type. Types, constructors and relation variables share one id space, assigned in operator log order: `add type T = A \| B` takes ids for `T`, `A`, `B` in that order, a record type takes its type id and one constructor id, `add constructor` takes one, `add relation r` takes one for `r`. The ids are a function of the log and are never reused. Every instantiation of a parametric sum shares its constructor ids. Engine-defined sums have fixed ids below 1024: the prelude's fixed per edition, those of the catalog modules per catalog version ([catalog.md §4.6](catalog.md#46-constructor-ids-of-engine-defined-sums)). |
| heap | The pages holding every row version of one constructor relation, keyed by (relation variable, constructor id). Two relation variables of one type have separate heaps. |
| heap id | The number of one heap, current or history (§7.5), assigned when the heap is created and never reused. |
| TID | The address of a row version within its heap: a page id and a slot. |
| constructor-set index | A B-link tree or hash index over the rows of a set of constructors of one relation variable. |
| entry | An index element `(key, ctor, tid)`. |
| xid | A 64-bit transaction id, never reused. |
| LSN | A byte position in the redo log. |
| commit position | A `Position { epoch, lsn }`: the primary epoch and the LSN of a transaction's commit record. |
| sum | A relation variable with all its constructor heaps and indexes. |

A sum's key is always covered by a constructor-set index over every
constructor of the sum, called its key index.

Rulings: substrate/new-engine, encoding/native, engine/guarantees-only, engine/own-pager, engine/embedded-server, engine/catalog-relations, project/workload/embedded-oltp, project/scale/out-of-core, buffer/platform/linux-first

## 2. File and pages

### 2.1 One file

A database is one data file, whatever its schema, plus its redo log
(§3) as segment files in a sibling directory named after the file:

```
app.adt        the data file
app.adt-log/   the log segments
```

Every heap, index,
free space map, visibility map, commit log page and catalog page is a
set of pages in it. Page ids form one dense space over the whole file;
the buffer pool (§4.1) and every TID use that space. Adding a
constructor adds an empty heap; removing one frees its pages once its
heap is dropped (§6.8).

Free pages are tracked by an allocation bitmap of one bit per page,
held in bitmap pages of the file and in memory while the database is
open. Freed pages are reused for later allocations. The file never
shrinks except by an explicit compaction (§10.1).

### 2.2 File header

The file header is kept as two alternating copies, on pages 0 and 1.
Each copy carries a sequence number and a checksum over the copy (in an
encrypted file, the header stays in clear and keeps its checksum). An
update writes the copy not holding the highest valid sequence number,
with that number plus one, and forces it; opening reads both and uses
the copy that verifies and has the higher sequence number. A torn
header write therefore leaves the other copy intact. Data pages start
at page 2.

The header holds a magic number, the file id, the lowest and
highest page format versions present, the finalization state of a
format upgrade ([catalog.md](catalog.md)), the location of the last
checkpoint record (§3.4), the root of the catalog, the encryption
parameters when the file is encrypted (§4.4), the page size (§2.4),
the commit log truncation bound (§5.6), whether page writes are
atomic on the device (§3.1), and the current primary epoch (§9.6).

> Open: the byte layout of a header copy, which no ruling fixes.

### 2.3 Page header

Every page starts with a header. A plaintext page has:

```
offset  size  field
0       8     lsn       LSN of the last log record applied to the page
8       4     checksum  CRC-32C over the page with this field zeroed, seeded with the page id
12      2     format    format version of the page's contents
14      2     kind      heap fixed-slot, heap slotted, index leaf, index inner, hash bucket,
                        free space map, visibility map, allocation bitmap, commit log, catalog
16      4     owner     heap id of a heap page, index id of an index page, 0 otherwise
20      2     flags     all-visible (heap), half-dead (index), ...
22      2     reserved
24            kind-specific contents
```

Each page states its own format version, so one heap may hold pages of
several row formats at once while each page stays uniform (§2.5).

An encrypted page replaces the checksum by an authentication tag and
adds a nonce and a data key version (§4.4).

> Open: the offsets of `kind`, `owner` and `flags`; only the LSN, the 32-bit checksum and the per-page format version are fixed by rulings.

### 2.4 Page size

The page size is a power of two from 4 KiB to 64 KiB, 4 KiB by
default. It is fixed when the file is created and recorded in the file
header; every page of the file, data and index alike, has that size.
Changing it is a compaction into a new file (§10.1).

### 2.5 Format versions

Rows and pages are little-endian. Index keys use their own
order-preserving encoding (§6.2), independent of byte order.

A release of the engine reads every earlier page and log format and
writes the newest. A page written in an old format is decoded by the
old decoder and is rewritten in the newest format only when it is
next written. The file header records the range of formats present, so
a decoder can be retired once no page of its format remains. Until an
upgrade is finalized a new release writes only formats the previous
release reads ([catalog.md](catalog.md)).

The same mechanism carries schema evolution inside a heap: a schema
operator that changes a constructor's row format gives the heap a new
format version, and pages are converted lazily as they are written
(§5.8).

### 2.6 Corruption detection

The engine promises crash safety and detection of corrupted pages. Its
fault model is crash stop, torn writes of any write larger than the
device's atomic unit, bit rot, latent sector errors, misdirected writes
and lost writes. A page that fails its checksum or tag is never
returned as data. A lost write is detected during recovery, and by
`verify` and the background scrub, which compare each page's LSN with
the page records of the retained log; a lost write whose records have
left the retained log, on a page not read by recovery, is not
detected.

| Fault | Detected by |
| --- | --- |
| Bit rot, latent sector error | Checksum (or tag) mismatch on read |
| Misdirected write | The page id seeds the checksum; a page read from the wrong place fails it |
| Torn write | Checksum mismatch; repaired from a full-page image in the log (§3.1) |
| Lost write | Page LSN older than the retained log says it must be, in recovery (§3.5) and in `verify` and scrub ([catalog.md](catalog.md#111-verification)) |

A page that fails verification on read quarantines the heap or index
holding it (§10.3).

Rulings: layout/files/single-file, buffer/cache/vm-reserve/global-pid, layout/alloc/bitmap, layout/files/shrink/reuse-only, layout/checksum/in-page-32, layout/endian/little, layout/format-version/read-old, layout/heap-format/page-version, layout/header/version, buffer/faults/corrupt-detect, buffer/faults/stale-page/lsn-check

## 3. Log and recovery

### 3.1 Redo-only logging

The log is redo-only. Old row versions stay in their heaps (§5), so an
aborted transaction's versions are invisible by the commit log and
need no undo; vacuum removes them later. A record describes a change
to one page, or a transaction or system event:

| Record | Contents |
| --- | --- |
| page change | page id, the page's LSN before the change, the change |
| full-page image | page id, the whole page |
| commit | xid, wall-clock time of commit; the record's position is the commit position |
| abort | xid |
| xid allocation | the next xid reserved |
| schema operator | an operator of a migration, and the schema version the migration produces |
| stage transition | an online schema state change (§7.6) |
| safe snapshot marker | a snapshot no read-write transaction can make unsafe (§7.4) |
| logical change | inserted and deleted rows of one constructor relation that has a registered consumer (§9.8) |
| checkpoint | redo LSN and dirty page table (§3.4) |

A retag, a split or any multi-page change is several page records in
one transaction; it is atomic because its versions become visible only
through its commit record.

The change that dirties a clean frame, the change that sets the
frame's `rec_lsn` (§3.4), logs a full image of the page instead of a
page change record. Every write of a page to the file therefore
writes a state whose last full image lies at or after the page's
`rec_lsn`, and so at or after the redo LSN of any checkpoint taken
while the page was dirty. Recovery restores a torn page from that
image.

When the device and file system guarantee atomic writes of a whole
page, no full-page images are logged outside a backup: a page write
either lands whole or not at all, so no page can be torn. The engine
detects the guarantee when it opens the file (for example from the
atomic write unit `statx` reports for direct I/O) and records it in the
file header; a file opened where the guarantee does not hold logs
images again from its next change of each clean frame. While a backup
runs (§9.9), full-page images are logged whatever the device
guarantees, since the backup's page copy can be torn.

Xids are allocated from a counter whose reserved upper bound is logged
before use, so the counter never goes back after a crash, even for
xids of transactions that abort. Constructor ids need no counter: they
follow from the operator log (§1.2).

A migration whose operators need staged jobs (§7.6) writes the record
naming its schema version only when its last staged job completes; a
failed job discards the pending migration, and the operator log is
unchanged.

### 3.2 Log blocks

The log is a sequence of preallocated segment files in the database's
log directory (`app.adt-log/` beside `app.adt`), named by the LSN of
their first byte. The log is written with direct I/O in whole blocks of the device's
logical block size. Each group commit (§3.3) is one batch of whole
blocks, padded after its last record; a block that has been
acknowledged is never written again. Batches are numbered by a
sequence that increases by one per batch. The blocks of a batch are
written and forced together, and the next batch is written only after
that force has completed.

```
log block
  0   u64  lsn of the block's first byte
  8   u64  batch sequence number
  16  u32  number of blocks in the batch
  20  u32  index of this block within the batch, from 0
  24  u32  epoch of the primary that wrote it (§9.6)
  28  u32  crc32c over the whole block with this field zeroed
  32  u16  bytes used
  34  u16  reserved
  36       records; a record may continue into the next block
```

In an encrypted database each log block is sealed as a page is (§4.4):

```
encrypted log block
  0     u64  lsn                     in clear
  8     u64  batch sequence number   in clear
  16    u32  number of blocks        in clear
  20    u32  block index             in clear
  24    u32  epoch                   in clear
  28    u32  data key version        in clear
  32    ...  ciphertext of bytes used and records
  end-40     24-byte nonce
  end-16     16-byte Poly1305 tag
```

XChaCha20-Poly1305 with a fresh random 192-bit nonce per block write;
the associated data is the file id and every clear field; the tag
replaces the CRC-32C.

Log segments are preallocated and zero-filled before use, so a write
inside a segment changes no file size, and the force at commit is
`fdatasync`. Creating or recycling a segment file is followed by an
`fsync` of the directory.

### 3.3 Group commit

A committing transaction appends its commit record and waits until the
log is durable past it. The first waiter becomes the leader: it writes
and forces every block up to the latest appended record, then wakes
every transaction whose commit record is now durable. Others wait for
the next leader. Under the quorum durability level (§9.5) the leader
also waits for replica acknowledgements of the batch, so one round
trip serves the batch.

### 3.4 Checkpoints

Checkpoints are fuzzy and never stop writers. Each dirty frame records
the LSN that first dirtied it (`rec_lsn`). A checkpoint record holds
the dirty page table and the redo LSN, the minimum `rec_lsn` over it;
the file header is updated to point at the record. Log before the redo
LSN is no longer needed for recovery and its segments may be recycled
once replication, archiving and change consumers no longer need them
(§9).

```
checkpoint record
  u64 redo_lsn = min(rec_lsn)
  u32 n
  (u64 page_id, u64 rec_lsn) * n
```

Background flushing writes dirty frames continuously. When the log
written since the redo LSN nears its configured bound, the flusher
writes the frames with the oldest `rec_lsn` first, so a page dirtied
over and over cannot pin the log.

A dirty frame is written only after the log is durable to the frame's
page LSN. A written frame is counted clean, and drops out of the dirty
page table, only after an `fdatasync` of the data file covers the
write.

### 3.5 Recovery

Recovery reads the checkpoint record named by the file header and
replays the log from its redo LSN to the end of the log. For each
record naming page `p`:

| Condition | Action |
| --- | --- |
| Full-page image | Install the image; the page LSN becomes the record's LSN |
| `p.lsn >= record.lsn` | Skip; the change is already on the page |
| `p.lsn == record.prev_lsn` | Apply the change; the page LSN becomes the record's LSN |
| `p.lsn < record.prev_lsn` | Lost write: quarantine the page's heap or index (§10.3) |

Replay verifies a page when it first reads it. A page that fails
verification is not corruption if the next record replay applies to
it is a full-page image; otherwise it quarantines the page's heap or
index (§10.3).

The end of the log is found by batch. A batch is accepted when all of
its blocks are present, each verifies, and they agree: the same
sequence number, block count and epoch, indexes 0 to count minus 1,
and LSNs that continue one another and the previous batch. Its
sequence number must be one more than the previous batch's. Replay
stops at the first batch that is not accepted. That batch and every
block after it are discarded: none of their commits was acknowledged,
since a commit is acknowledged only after its whole batch is forced.
Before the first new write, the log tail after the last accepted batch
is zeroed and forced, so blocks of a discarded batch can never be read
as a continuation later. A block that fails verification followed by
an accepted batch with a higher sequence number is corruption of the
log, since an earlier batch is forced before a later one is written,
and stops recovery.

Recovery reads both log and pages with direct I/O, so it reads what
is on the device and not a stale kernel cache.

### 3.6 Write and flush failures

A failed write or `fdatasync` of the log or a page is not retried. The
engine crashes and recovers from the log: the failed pages are rebuilt
from their last durable image plus replay. Commits that waited on the
failed force were not acknowledged.

### 3.7 Bulk loads

A load fills new heap and index pages in private memory and logs each
filled page once as a full-page image, with no per-row records. The
images are what a physical replica (§9.1) replays, so replicas receive
the loaded rows. The load's change set is one transaction; its pages
become visible only through its commit record
([interop.md](interop.md#4-loading)). A load into a constructor
relation of a registered feed also writes logical change records of
the loaded rows (§9.8).

Rulings: engine/redo-mvcc, layout/torn/fpw, buffer/io-path/log-align/slot-pad, buffer/force/fdatasync, buffer/group-commit/leader, buffer/checkpoint/fuzzy, buffer/checkpoint/log-pin/flush-oldest, buffer/fsync-fail/panic-replay, interop/load/wal/page-images

## 4. Buffer and I/O

### 4.1 Page cache

The buffer pool reserves anonymous virtual memory the size of the
page-id space; page `p` lives at `base + p * page_size`. A miss reads the
page into its own address; eviction writes it if dirty and releases
the memory with `madvise(MADV_DONTNEED)`. Index entries, chain links
and heap directories all name pages by page id, so any number of
references may name one page.

Each page has a 64-bit state word:

```
state[p]
  bits 56..63  0 unlocked | 1..252 shared count | 253 exclusive | 254 marked | 255 evicted
  bits  0..55  version, incremented on every exclusive unlock and on eviction
```

Optimistic readers (§6.4) read the version, read the page, and
revalidate the version. A page in flight for I/O is held exclusive
until its completion is processed, so cancellation of the requesting
work never frees memory the kernel writes into.

> Open: the replacement policy, which no ruling fixes; the marked state supports a clock with a one-bit reference count.

### 4.2 Memory

Memory is split into a fixed page pool, sized at open, and per-query
memory. Each query obtains arenas on demand within a per-query budget
and frees them whole when it ends; an allocation past the budget is an
operating error of that query. The planner sizes hash builds, sorts
and groupings against the grant the query was admitted with
([catalog.md](catalog.md)), and operators that exceed it spill to
temporary pages of the pool, then, when the pool has none to spare, to
spill files in the spill directory. Spill files of an encrypted
database are encrypted under a key generated per process and never
stored ([security.md](security.md#91-what-is-encrypted)).

### 4.3 I/O path

Data pages and the log are read and written with `O_DIRECT`: the pool
is the only cache, and write errors return from the write itself. A
commit still forces the device cache with `fdatasync` (§3.2).

The pager issues I/O through one submit and complete interface. On
Linux it is backed by `io_uring`; where `io_uring` is unavailable, by a
pool of threads doing blocking `pread` and `pwrite`. Other systems use
the thread pool with their own direct I/O flag. The deterministic
simulator is a third backend
([implementation.md §2.4](implementation.md#24-runtime-and-io-loop)).

### 4.4 Encryption at rest

When a database is encrypted, the pager encrypts every page as it is
written and decrypts it as it is read; pages are plaintext in the
pool and ciphertext in the file. The cipher is XChaCha20-Poly1305:

```
encrypted page
  0     u64  lsn                 in clear
  8     u48  page number         in clear
  14    u16  format              in clear
  16    u16  kind                in clear
  18    u16  reserved
  20    u32  data key version    in clear
  24    ...  ciphertext of the rest of the page
  end-40     24-byte nonce
  end-16     16-byte Poly1305 tag
```

- Each write draws a fresh random 192-bit nonce and stores it in the
  page. The nonce does not depend on the LSN: a page whose frame is
  already dirty may be written again with kill bits (§6.4) set since
  its last write and no new LSN. Kill bits set on a clean frame log a
  full-page image first (§3.1).
- The associated data binds the file id (from the file header), the
  page number, the LSN and the data key version, with the other clear
  fields. A page moved to another address or another file, or with its
  LSN altered, fails the tag.
- A page replaced by an older valid ciphertext of itself carries an
  older LSN under a valid tag. Recovery detects it by the LSN check
  against the log (§3.5); outside recovery, detecting such a rollback
  is out of scope.
- The tag replaces the CRC-32C checksum. A tag that fails is a
  corrupt or forged page and is handled as any corruption (§2.6).
- A write encrypts into a staging buffer that is then written; the
  frame stays plaintext, so optimistic readers never see ciphertext.
  A read decrypts in place in the frame before the frame is published.

Data keys are wrapped by a key-encryption key and named by version;
rotating the data key makes new writes use the new version, and older
pages move to it as they are rewritten. Key sources, rotation and the
treatment of backups are in [security.md](security.md#9-encryption-at-rest).

Log blocks use the same construction (§3.2).

Rulings: buffer/cache/vmcache, buffer/cache/vm-reserve/global-pid, buffer/memory/split, project/arch/alloc/budgeted, buffer/io-path/direct, buffer/async/uring-fallback, buffer/platform/linux-first, security/encrypt/page, security/nonce/random, security/integrity/aead, security/encrypt/io-cost/staging

## 5. Heaps and versions

### 5.1 One heap per constructor

Each constructor of a relation-level sum has its own heap. A heap holds
rows of one schema only. A scan of a constructor relation reads its
heap; a scan of the whole sum is the disjoint union of its heaps'
scans, and an ordered scan of the sum merges ordered scans of each heap
on the fly.

Each heap has a directory naming its pages, recorded in the catalog,
and a free space map and visibility map of its own.

> Open: the structure of a heap's page directory (a page list or a tree), which no ruling fixes.

### 5.2 Page arrangement

The heap's schema picks the page arrangement. A constructor whose
fields all have fixed width (including value-level sums stored at
maximum payload, §5.5) has fixed-slot pages; any other constructor has
slotted pages.

Fixed-slot page:

```
page header (24 B)
u64 used_slots[ceil(n / 64)]     bit i set: slot i holds a version
u16 slot_flags[n]                the version flags of slot i (§5.3)
slot[0] slot[1] ... slot[n-1]    slot i at a fixed offset, width w = 24 + body width
```

A TID's slot number is the array index. Freeing a slot clears its bit
and its flags; no compaction occurs in a fixed-slot page.

Slotted page:

```
page header (24 B)
u16 slot_count
line pointer[0] ... line pointer[slot_count-1]   8 B each: u16 offset, u16 length, u8 state, u8 reserved, u16 flags
free space
tuples, packed from the end of the page
```

Offsets and lengths are 16 bits, which covers every page size up to
64 KiB; a tuple is at most a page minus its header. Line pointer states
are unused, normal, dead (pruned, still named by
index entries) and redirect (names the next live version of a
heap-only chain on the page). Tuples move during in-page compaction
without changing their TID. The line pointer's `flags` are the version
flags of §5.3.

A TID is 64 bits: a 48-bit page id and a 16-bit slot. In an index
entry it is stored big-endian.

### 5.3 Version header

Every version starts with a 24-byte header:

```
0   u64  xmin    inserting transaction
8   u64  xmax    deleting or superseding transaction, 0 when none
16  u64  next    TID of the successor version, in this heap or, when moved, another heap of the sum
24       body, at a fixed offset for every row of the heap
```

Each version also has a `u16` of flags held in its slot, not in the
header: in the line pointer of a slotted page, and in the page's
`slot_flags` array of a fixed-slot page (§5.2). A flag change is a
page change and is logged as one.

| Flag | Meaning |
| --- | --- |
| moved | The successor is in another heap of the sum; `next` names it (§5.10) |
| retagged-in | This version was inserted by a retag |
| hot-updated | The successor is a heap-only version on this page |
| heap-only | No index entry names this version |

There is no attribute count, header length or null bitmap: the heap's
schema and the page's format version fix the body layout, and no field
is NULL. There are no commit-status bits; readers consult the commit
log (§5.6). A transaction's command ids for its own versions live in
its local state.

### 5.4 Row body

Fields are placed by the engine, not in declaration order: fixed-width
fields are ordered by decreasing alignment to remove padding, then a
table of `u16` end offsets for the variable-width fields, then the
variable-width bytes. The catalog records, per heap format version,
each field id's offset, so a rename or a move of a field changes no
stored byte.

```
type Order = { paid: Bool, id: I64, qty: I16, total: Decimal 2 }
body:  0 total (16)  16 id (8)  24 qty (2)  26 paid (1)     27 B, padded to 32
```

For a variable field `j`, the bytes are `[end[j-1], end[j])`, with
`end[-1]` the start of the variable area.

### 5.5 Value-level sums in rows

A field whose type is a sum is stored in the layout its shape allows:

| Shape | Layout |
| --- | --- |
| Every constructor fixed width and the largest payload within an engine constant | tag, then a payload area the size of the largest constructor's payload |
| Otherwise | tag in the fixed part; the payload as a variable-width field |

The stored tag is the constructor id (§1.2), the same number the wire
carries. Where the tag lies in the variable-width part of a row it is
an unsigned LEB128 varint. Where it lies in the fixed part (a fixed-slot
page, or the fixed part of any row) it is a little-endian unsigned
integer of 1, 2 or 4 bytes, the smallest that holds the largest
constructor id of the type when the page format is created; an id
outgrowing it gives the heap a new format version. Nullary constructors store only the tag.
A one-constructor sum (a record) stores no tag.

```
type Status = Draft | Sent { at: Instant } | Paid { at: Instant }
status field, fixed: u16 tag | pad 6 | i64 at       16 B; Draft leaves 8 B unused
```

Adding a constructor whose payload is larger, or whose id outgrows
the format's tag width, gives the heap a new format version; rows are
rewritten lazily as their pages are written (§5.8).

### 5.6 Transaction ids, the commit log and snapshots

Transaction ids are 64 bits and never wrap, so versions are never
frozen. The commit log holds 2 bits per xid (in progress, committed,
aborted) in commit log pages of the file. Commit positions are not in
the commit log; for the time-travel window a separate map gives them
(§7.5). A snapshot is:

```
snapshot { xmin: u64, xmax: u64, in_progress: [u64] }
```

A version's writer `x` counts as committed for snapshot `s` when
`x < s.xmax`, `x` is not in `s.in_progress`, and the commit log says
committed. The in-progress test precedes the commit log lookup.
Readers never write heap pages to cache a writer's status.

The commit log is truncated from below. Once every xid below a bound
`b` has resolved, and vacuum has removed from every heap every version
written by an aborted xid below `b`, the commit log prefix below `b` is
dropped and every xid below `b` reads as committed. The bound is
recorded in the file header (§2.2). Vacuum clears the `xmax` of a
version whose deleter aborted (§7.5), so no remaining version names an
aborted xid below the bound.

### 5.7 Version chains

Chains run oldest to newest: an update writes a new version and points
the old one's `next` at it; index entries name the version they were
inserted for (version TIDs). A reader wanting the newest version walks
forward; an older snapshot stops at the first version it can see.

An update is heap-only when it fits on the same page and changes no
field that is a key field of an index whose constructor set contains
this heap's constructor. Field values are compared bytewise, so a
value-level sum field that changes constructor is a change. A heap-only
update adds no index entries; any other update inserts a new entry
into every index whose set contains the constructor. Creating an index
over a constructor set handles broken heap-only chains in those heaps
only.

### 5.8 Row format changes

Each page holds rows of one format version. A schema operator that
changes a constructor's fields or widens a value-level sum gives the
heap a new format version. Reads decode a page by its own version;
the first write to a page converts every row on it to the newest
version. A converted page that no longer fits its rows moves the
overflow rows to other pages as updates, with new TIDs and new index
entries.

### 5.9 Large values

A value too large for a page goes out of line to a large-value store
belonging to the sum, shared by all its constructor heaps; the row
holds a fixed-size pointer to it. A retag (§5.10) moves the row
between heaps and keeps its pointers, so no out-of-line value is
copied. Vacuum of the sum frees chunks no live or retained version
references.

### 5.10 Retag

Retag keeps the row's key and moves the row atomically into another
constructor's heap. For the update below on the `EmailPass` row with
key `k`:

```
update i in identities where i.id == k
set match i
  | EmailPass e -> Google { user: e.user, gid: g }
  | Google o    -> o
  | Apple a     -> a
```

the writing transaction `T`:

1. Inserts the new version in the `Google` heap at `tid'` with
   `xmin = T` and the retagged-in flag.
2. Sets `xmax = T`, the moved flag and `next = tid'` on the old version
   in the `EmailPass` heap.
3. Inserts an entry `(k, Google, tid')` into every index whose
   constructor set contains `Google`. The old entry `(k, EmailPass,
   tid)` stays in every index containing `EmailPass` until vacuum.
4. Checks each new entry against range locks as an insert of a
   `Google` row (§7.2).

A retag is never heap-only. Snapshots older than `T` find the old
version through the old entry; newer snapshots find the new one. A
uniqueness probe that finds the old entry skips it because its
version's deleter is `T` itself.

A transaction that finds the old version being updated, deleted or
retagged by `T` waits for `T`. If `T` commits, the waiter fails with a
serialization failure; if `T` aborts, the waiter proceeds on the old
version. A waiter never re-evaluates its statement on a successor.

A walk of the chain to the newest version (vacuum, `verify`) follows
`next` across heaps: page ids are global, so the successor's TID names
its page in the other heap. A transaction that retags a row from `A`
to `B` and back to `A` leaves each version's `next` naming exactly its
own successor.

Rulings: storage/table-per-ctor, storage/sum-scan/merge-append, encoding/native-cost/union-index, layout/slots/by-schema, layout/fill/fsm, layout/tuple-header/lean, layout/field-order/packed, layout/var-fields/offset-table, layout/vsum/by-shape, layout/vsum-tag/varint, layout/vsum/widen/rewrite, layout/heap-format/page-version, layout/large/toast, layout/large/retag/per-sum, engine/in-heap, mvcc/xid-clog, mvcc/xid-width/xid64, mvcc/status/clog-lookup, mvcc/o2n, mvcc/version-tid, mvcc/hot/per-heap, union-index/retag/move, engine/retag-chain/forward-pointer, engine/retag-snapshot/two-entries, mvcc/link/key-relookup, layout/succ/key-probe, mvcc/link/reinsert/in-flag

## 6. Indexes

### 6.1 Constructor-set indexes

Every index, unique constraint, key and reference target is one
mechanism: an index declared over a constructor set `S` of one
relation variable, holding an entry `(key, ctor, tid)` for every
indexed version of a row whose constructor is in `S`. The key index
ranges over every constructor; `unique (gid) in Google` over
`{Google}`; a reference typed `Key identities.OAuth` targets an index
over `{Google, Apple}`.

Entries sort by key, then constructor id, then TID. All entries of one
key are adjacent, so a uniqueness probe reads one range; within a key,
one constructor's entries are adjacent, so a probe for
`Key identities.Google` seeks `(k, Google)`.

```
key index of identities:
  (7, EmailPass, (12,3))  (7, Google, (4,1))  (8, Google, (4,2))
```

The constructor id is encoded as an order-preserving, prefix-free
varint, `ovarint`, not LEB128: an id below 240 is one byte, the id; a
larger id is the byte `239 + n` followed by the id as `n` big-endian
bytes, `n` from 1 to 4 the fewest that hold it. The whole entry is one
byte string compared with `memcmp`:

```
entry bytes = encode(key) ++ ovarint(ctor) ++ be48(page) ++ be16(slot)
```

### 6.2 Key encoding

Index keys are byte strings whose `memcmp` order is the key order and
whose equality is the language's equality: values equal in the
language encode to identical bytes, and the encoding decodes back to
the value.

| Type | Encoding |
| --- | --- |
| `I8` to `I64` | Big-endian two's complement of the type's width, sign bit inverted |
| `F32`, `F64` | The canonical value (no -0.0; one NaN) as big-endian IEEE bits; if the sign bit is set every bit is inverted, otherwise only the sign bit; NaN sorts above every other value |
| `Decimal` | The scaled integer coefficient, encoded as a signed integer of its storage width |
| `Instant`, `Date`, `Time`, `DateTime`, `Duration` | Their integer representation, encoded as the signed integer of its width |
| `ZonedDateTime` | The `DateTime` encoding, then the zone name as `Text`; the type has no order, so the encoding serves equality and uniqueness only |
| `Period` | The month count, then the day count, each as an `I32`; equality and uniqueness only, as for `ZonedDateTime` |
| `Text` | NFC UTF-8 bytes, each `00` written `00 FF`, terminated by `00`; the order is code point order |
| `Bytes`, `SortKey` | As `Text`, without normalization |
| `Key r` | The encoding of the target's key |
| newtype | The encoding of its representation |
| record | Its fields' encodings concatenated in field order |
| sum value | The constructor's order key, escaped and terminated as `Text`, then its payload as a record |

A sum's tag is encoded by an order key: a short byte string per
constructor kept in the catalog. Order keys follow the sum's declared
order; a constructor inserted between two others gets a string between
theirs, so no stored key changes; reordering rewrites only the keys
holding the moved constructor. An index scan of an ordered sum field
is therefore in the language's order. Optionality is a constructor, so
where `None` sorts is its place in the declared order.

```
type Status ordered = Draft | Sent { at: Instant } | Paid { at: Instant }
order keys: Draft 40, Sent 80, Paid C0
Sent { at: t }  ->  80 00 | be64(t xor 2^63)
```

A collated order is an expression index over a collation function's
sort key ([derived.md](derived.md#5-computed-fields)); `Text` keys
themselves compare by code point only, so every key decodes.

### 6.3 Indexes on value-level sum fields

A value-level sum field may be indexed three ways:

| Declaration | Key | Use |
| --- | --- | --- |
| `index (status)` | the whole value, as §6.2 | equality, constructor prefixes, ranges on ordered sums |
| `index (tag(status))` | the order key only | constructor tests |
| `unique (status.Paid.ref)` | one constructor's field; entries only for rows holding that constructor | used by an arm whose pattern names the constructor |

An index on a constructor field is used only where a query's arm
pattern names that constructor; the implication is syntactic and needs
no predicate prover. Expression indexes and the derived indexes of
full-text and spatial search are B-link trees over relations derived
by a function ([derived.md](derived.md#5-computed-fields),
[interop.md](interop.md)) and inherit everything in this section.

### 6.4 B-link trees

Ordered indexes are B+-trees with right-links and high keys (B-link
trees). Every page but the rightmost of its level carries a high key;
a descent whose key exceeds a page's high key moves right, so a split
holds latches on one level at a time. Every entry is unique because
`(ctor, tid)` trails the key. Sequential keys append at the right edge.

Latching is optimistic lock coupling on the page's state word (§4.1):
a reader records the version, reads, and restarts from the root if the
version changed or the page was locked; a writer takes the word
exclusive and increments the version on release. A descent never
writes a state word; a scan that sets kill bits takes the leaf's word
exclusive for the write, as a writer does. Optimistic reads load page bytes through atomic loads
into local copies and bounds-check every length and offset read before
use; no reference to a page another thread may write is formed.

A leaf stores equal keys once followed by a sorted array of
`(ctor, tid)` pairs. Deduplication runs lazily, when a leaf would
otherwise split.

```
posting tuple:  key | n | (ovarint ctor, tid 8 B) * n
user 3: [(EmailPass,(12,3)), (EmailPass,(40,1)), (Google,(4,1))]
```

Dead entries are removed between vacuum passes in two ways. A scan
that finds an entry's version dead to every snapshot sets the entry's
kill bit. Before splitting a leaf, an inserter first deletes killed
entries, then groups the leaf's duplicate entries by `(ctor, page)`,
visits those heap pages, and deletes entries whose versions are dead to
all; a retag's old entry beside its new one is such a duplicate. Kill
bits are not logged as records; setting one on a clean frame dirties
it, and so logs a full-page image first (§3.1).

Underfull pages are never merged. An empty page is unlinked from its
parent, then from its siblings, and recycled once no snapshot older
than its deletion remains. The rightmost page of a level is never
deleted.

> Open: split point choice and key compression, both deferred.

### 6.5 Hash indexes

A constructor-set index whose every use is an equality probe may be a
hash index. A bucket entry is `(hash u32, ctor, tid)`; equal hashes are
rechecked against the version in the heap the entry names. A hash
index can enforce uniqueness, has no ordered scan and serves no sum
scan merge.

> Open: SIREAD locks on a hash index take the probed hash value and constructor set, as an equality range in §7.2; no ruling states the granularity.

### 6.6 Uniqueness checks

An insert into a unique index probes its key range. For each equal
entry it fetches the version from the heap the entry's constructor id
names and checks it against the latest committed state, not the
inserter's snapshot:

A conflict is reported as `KeyViolation` in the key index and as
`Violated`, carrying the unique as its constraint, in the index of a
unique.

| Version found | Outcome |
| --- | --- |
| Live, committed, visible to the inserter's snapshot | Conflict |
| Live, committed after the inserter's snapshot | Serialization failure |
| Inserted by a transaction in progress | Wait for it; serialization failure if it commits, proceed if it aborts |
| Deleted by a transaction committed before the inserter's snapshot | Skipped |
| Deleted by a transaction committed after the inserter's snapshot | Serialization failure |
| Deleted by a transaction in progress, the version visible to the inserter's snapshot | Wait for it; serialization failure if it commits, conflict if it aborts |
| Dead to every snapshot, or deleted by the inserter itself | Skipped |
| Inserted by another write of the same statement | Conflict, reported at the statement boundary |

A key may thus hold several entries, at most one of them for a live
version. No other constructor's heap is locked.

### 6.7 Index-only scans

Each heap has a visibility map with one all-visible bit per page, set
by vacuum and cleared by any write to the page. An index-only scan
reads `vm[heap][page(tid)]` for each entry, `heap` the heap of the
entry's constructor in the index's relation variable, and fetches the version
only when the bit is clear. A query that needs only key fields and the
constructor, such as a match whose arms read no other field, is
answered from the index. A heap whose constructor is rarely written
stays all-visible while other constructors of the sum change.

### 6.8 Building and rebuilding

An index over several heaps is built by scanning and sorting each heap
into a run of entries independently and in parallel, then merging the
runs into leaves filled left to right, building each level from the
first keys of the level below. A heap already sorted by the key skips
its sort. A unique index reports adjacent equal keys in the merge as
violations.

A split or merge of constructors moves rows to the new constructors'
heaps and, in every index of the sum, deletes the moved rows' old
entries and inserts entries naming the new constructor ids, batched by
key order. One heap per constructor holds literally: no constructor
owns another's heap.

A load chooses, per load, how to maintain each index: a bulk build from
sorted runs into an index over an empty constructor set, a sorted merge
of the load's entries in key order into a populated index, or retail
inserts.

An index is rebuilt either online, by building a new index from sorted
runs of each heap while writes continue, catching up with writes made
meanwhile, then swapping, staged as a schema job (§7.6); or offline,
with writes to every constructor of the index's set blocked. One
constructor's entries are spread through the tree, so an index is
always rebuilt whole.

Removing a constructor applies only to a constructor with no live
row, but older snapshots, the time-travel window and index entries may
still name its versions. The remove keeps the heap as history until
vacuum has cleared its index entries and no snapshot or retention
window covers its rows; then the heap is dropped and its pages freed.

Rulings: index/ctor-set, btree/entry-order/after-key, btree/ctor-width/uvarint, btree/keyenc/memcmp, btree/keyenc/canon/canonicalize, btree/varlen/escape-term, btree/keyenc/lossy/code-point, btree/sum-key/tag-payload, btree/sum-tag/order-key, btree/sum-field/whole, btree/sum-field/by-tag, btree/sum-field/ctor-field, btree/sum-field/implied/arm-scoped, btree/structure/blink, btree/latch/olc, btree/latch/racy/atomic-reads, btree/posting/posting-pairs, btree/cleanup/kill-bit, btree/cleanup/bottom-up, btree/merge/empty-only, btree/hash/eq-unique, engine/wait-inserter, mvcc/vm-per-heap, btree/ios/vm-per-heap, btree/build/merge-runs, btree/ctor-rewrite/rewrite, interop/load/index-path/bulk-build, interop/load/index-path/sorted-merge, interop/load/index-path/retail, ops/reindex/online, ops/reindex/offline, union-index/drop-ctor/eager

## 7. Transactions

### 7.1 Serializable snapshot isolation

Every transaction is serializable; there is no weaker level
([transactions.md](transactions.md)). The engine provides it by
serializable snapshot isolation over the in-heap versions of §5:
readers read their snapshot and never block; a writer that would
create a dangerous structure of two consecutive rw-antidependencies
among concurrent transactions aborts one of them with a serialization
failure. A write that meets a row version committed after the writer's
snapshot (an update, delete or retag of a row a later commit changed,
or a wait on such a writer, §5.10) is a serialization failure, never a
key violation and never a re-evaluation on the newer version.

Serializability carries the rules that would otherwise need their own
locks:

- A constraint check, including an at-least-one check, is a read of
  its transaction, so two concurrent deletes that together violate it
  form a dangerous structure and one aborts. Concurrent checks cannot
  jointly admit a violation ([constraints.md](constraints.md)).
- A reference check reads its target, and a retag's recheck of
  referrers reads the referring range; a concurrent insert of a
  referrer typed by the old constructor and the retag conflict through
  those reads.

### 7.2 SIREAD locks

SIREAD locks record reads; they never block. A writer checks the locks
its writes cover and records a rw-antidependency for each.

| Lock | Taken by | Conflicts with |
| --- | --- | --- |
| Tuple `(heap, tid)` | reading a version | an update, delete or retag of that version |
| Index range `(index, lo, hi, ctors)` | an index scan or probe, over the gaps between the entries it saw | an insert of an entry in `[lo, hi]` whose constructor is in `ctors` |
| Heap `(heap)` | a sequential scan of a heap | any insert into that heap |
| Schema version `(v)` | every statement, for the schema version it runs at | the commit of a migration producing a later version |

Range locks are next-key ranges, not leaf pages. Each range lock
carries the constructor set the query read, as a set over the index's
constructor ids, so a query over `identities.(Google | Apple)` that
scans the key index does not conflict with an `EmailPass` insert into
the range:

```
from i in identities.OAuth where i.user == u select i.id
-- range lock on the index over user, ctors {Google, Apple}
```

Under memory pressure tuple locks promote to page locks and then to a
heap lock, and range locks promote to an index lock; promotion keeps
the union of the constructor sets.

A retag's new entries (§5.10) are checked against range locks as
inserts of rows of the new constructor, so a serializable scan of
`identities.Google` that saw no row with key `k` conflicts with a retag
of `k` into `Google`. Tuple locks on the old version conflict as for
any update; they are not copied to the new version.

Applying a migration writes the schema version, so a transaction whose
statements ran at version `n-1` and a migration producing version `n`
form a rw-antidependency, and a transaction that also writes is
serialized against the migration like any other writer.

A bulk load that builds an index over an empty constructor set
(§6.8) inserts no entries one by one. Its commit is checked as an
insert into every constructor heap it fills: it conflicts with every
heap lock on those heaps and every range lock, on any index over them,
whose constructor set contains one of them. Its constraint checks take
SIREAD locks as any read does.

A split or merge of constructors replaces the tuple, page and range
locks held on the rewritten heaps and their index ranges by one heap
lock on every heap the operator writes, so later writes to those
constructors conflict with the old readers.

### 7.3 Long readers and the watermark

Reclamation is bounded by one oldest-snapshot watermark, computed per
heap. A transaction whose constructor relations are known (a
registered query or transaction, whose plan names them) publishes its
snapshot only to the heaps it can read: the heaps its plan reads, plus
every heap read by the checks and delete actions of the relations it
may write. The set is recomputed when the schema changes. An
interactive transaction,
whose reads are unknown, holds back every heap. A report over
`invoices` does not hold back vacuum of `identities`. The watermark of
a heap also includes the oldest snapshot replicas report for it (§9.3).
Versions kept for the time-travel window are handled by §7.5.

### 7.4 Retry and safe snapshots

The server reruns a registered transaction on a serialization failure
before replying, and the client library reruns an in-process closure
([transactions.md](transactions.md#55-retry)); an interactive
transaction is never rerun by the engine.

Reads on the primary run under SSI and never wait for a safe snapshot.
Only a transaction declared `read_only deferrable` waits for one on
the primary: a snapshot with no concurrent read-write transaction that could still make it part of
a dangerous structure. It then runs without SIREAD locks and never
aborts with a serialization failure. The primary logs a marker for
each safe snapshot it finds; replicas read only at such markers (§9.2).

### 7.5 Reclamation

Vacuum runs in the background per sum, in phases:

1. Scan each constructor heap of the sum, skipping all-visible pages,
   and collect the TIDs of versions dead to every snapshot the heap's
   watermark admits, and of versions whose inserter aborted.
2. Make one pass over each index of the sum, deleting every entry whose
   `(ctor, tid)` is in the collected set.
3. Free the dead slots, update the free space map, set all-visible bits
   where every remaining version is visible to every snapshot, and
   clear `xmax` of versions whose deleter aborted.

The dead set is a radix tree keyed by heap id and page, each
leaf a bitmap of dead slots; a sum's heaps are disjoint subtrees. When
the set reaches its memory budget, phases 2 and 3 run and phase 1
resumes.

Between passes, any access holding a heap page exclusively prunes
versions on it that are dead to every snapshot: heap-only versions are
freed, and versions still named by index entries leave a dead slot for
vacuum's index pass. Pruning is logged.

With a time-travel window ([derived.md](derived.md#6-reading-the-past)), a
version deleted within the window and dead to every running snapshot
is moved by phase 3 into a history heap of its constructor instead of
being freed, so current heaps and indexes do not hold history. Each
history heap has a key index whose entries are `(key, commit position,
tid)`, the commit position being that of the version's inserter. A
read `at commit p` ([derived.md](derived.md#6-reading-the-past)) reads
at the latest safe snapshot marker at or before `p`, a function of `p`
listed in `sys::safe_snapshots`
([catalog.md](catalog.md#15-commit-times-and-safe-snapshots)), and takes no SIREAD
locks. It reads the current and
history heaps of the constructors it reads, probing the history key
index by key where the current read would probe the key index, and
counts a version's writer as committed when the writer's commit
position is at or before the marker; versions past the window are
removed from history heaps. For every xid committed within the window
a commit position map beside the commit log gives its commit position,
and the commit records' wall-clock times map a time to a position.

Vacuum's progress, triggers and budget are exposed as described in
[catalog.md](catalog.md#115-vacuum).

### 7.6 Schema changes

A schema operator is a write: it runs in a transaction and commits or
rolls back with the data writes beside it. Operators that need
intermediate states pass through them as staged states, each
compatible with the last ([evolution.md](evolution.md#34-staged-changes)):

| Object | States |
| --- | --- |
| constructor added by `add constructor` | declared (readers can type it, no writes) then writable, then deprecated, then removed when empty; `disable` returns a writable constructor with no rows to declared |
| constructor of a value-level sum added by `add constructor` | the same states; while declared, no field of any relation stores a value of it |
| constructor created by `add type` | writable from the start |
| product of split, merge, nest or unnest | writable when every source constructor is writable |
| index | write-only, backfilled, readable |

The write state of each constructor is a row of `catalog::ctor_states`,
keyed by (relation variable, constructor). Each state must be committed
and seen by every running transaction before the next begins. The user's transaction commits the first state;
a background job runs the rest and records its phase in the catalog.
The migration's schema version commits only when every staged job of
the migration completes; a failed job discards the pending migration,
the operator log is unchanged, and no inverse operators are appended.
Every check the migration's operators made when they applied reruns at
the commit of its last staged job, and a failure discards the pending
migration as a failed job does. A host function must be bound before
any staged job or plan needs it; an embedded open is refused, naming
the function, when a staged job or index build of a migration before
the shipped redefine of a host function reaches that function
([embedding.md](embedding.md)). Registered queries pinned to an
older schema version keep running through translation by the
operators' inverses while the staged change rolls out
([evolution.md](evolution.md#7-version-translation)).

Schema operators and stage transitions are log records (§3.1), applied
in log order with the data on recovery and on replicas.

### 7.7 Stored views

A stored view is held like a base relation, with heaps per constructor
and indexes, and is maintained in the transaction that changes its
inputs ([derived.md](derived.md#2-stored-views)). Maintenance uses the
compiler that checks constraints: each view gets a delta query per
input constructor relation, run over the transaction's change set; a
set-valued view keeps a derivation count per stored row and drops a
row when its count reaches zero. Delta queries probe base relations
through their indexes at the transaction's snapshot and keep no other
state, so a view is admitted only when every join reached from a delta
has an index. A bundle pinned before a view's redefinition reads the
view by evaluating the body of its own version, not the stored rows
([semantics.md §9.6](semantics.md#96-version-translation)).

Count and sum maintenance writes are commutative increments merged at
commit; the maintainer records no read, so two writers to one group do
not conflict unless one of them reads the aggregate. Integer overflow
of a merged value is detected at commit: the commit whose increment
would overflow the stored value fails with `Rejected { reason: Faulted { fault: ArithmeticOverflow } }` and
applies nothing.

Rulings: engine/serializable-only, engine/ssi, engine/write-skew/by-isolation, engine/retag-fk/by-isolation, mvcc/next-key, mvcc/ctor-tagged, mvcc/retag-phantom/as-insert, mvcc/split-locks/promote-heaps, mvcc/watermark, mvcc/long-reader/ctor-scoped, engine/retry/retry-block, engine/retry/safe-snapshot, mvcc/background, mvcc/on-access, union-index/vacuum/batched, btree/dead-set/radix, derived/time-travel/retention/history-heap, engine/transactional-ddl, engine/online-states, engine/ddl-online-txn/staged-job, engine/pinned-version, derived/ivm/delta-rules, derived/ivm/state/base-indexes

## 8. Execution

### 8.1 Execution model

A physical plan is a set of pipelines through which producers push
batches into consumers. A lowered `match` is a union of arm queries;
each arm is its own pipeline, `scan(heap) -> arm -> sink`, and the
union is the shared sink.

Pipelines start in a bytecode interpreter. Per pipeline the engine
extrapolates the remaining work from morsel progress and compiles hot
pipelines of registered queries to machine code when the estimate says
compilation pays.

> Open: the code generator backend, deferred.

Parallelism is morsel-driven: worker threads take (pipeline, morsel)
tasks from a dispatcher, steal work, and write to thread-local output
areas. A morsel never spans heaps; several small constructor heaps' arm pipelines are packed into one task
that runs them in turn. Calls from the server become tasks on the same
scheduler ([wire.md](wire.md)).

### 8.2 Batches

Batches between operators are Arrow record batches. A batch from a
constructor heap has that constructor's schema and no validity
bitmaps, since no field is NULL.

A value-level sum field is an Arrow dense union: an `i8` type-id
buffer, an `i32` offsets buffer, and one compact child per constructor.
Three shapes are special-cased:

| Sum shape | Batch layout |
| --- | --- |
| `Option T` (a nullary constructor and one other) | `T` with a validity bitmap; a cleared bit is the nullary constructor |
| Only nullary constructors | The tag vector alone |
| One constructor | Its payload, with no tag |

These are engine layouts only; the language still sees constructors.
Type ids in a batch are positions in the batch schema, which maps each
to the constructor's stable id. Nested results are Arrow lists of
structs.

A union holds at most 127 constructors. A sum with more is a union of
unions: the outer union's children are unions of up to 127
constructors each, nested as deep as needed. The batch schema's type
map still names each leaf constructor by its stable id
([wire.md](wire.md)).

### 8.3 Executing match

Where the constructor is known per pipeline, an arm runs over its own
heap's batches. Where it varies per row (a value-level sum field, a
probe of an index over several constructors, or a merge of several
heaps), the batch is split by a single pass over the tag into one
selection vector per constructor, each arm runs over its selection,
and results are written back to their positions.

Arms whose sources are the same index range are fused: one probe of
the index, and each entry `(key, ctor, tid)` is routed by its
constructor to a fetch from that heap and that arm. This is a physical
operator; the meaning of the match is still the union of its arms.

```
from i in identities where i.id == k
select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub

IndexScan key index (id = k)
  -> Route by ctor { EmailPass: Fetch(EmailPass heap) -> arm 0
                     Google:    Fetch(Google heap)    -> arm 1
                     Apple:     Fetch(Apple heap)     -> arm 2 }
```

Arms are pruned at compile time when the source's type excludes their
constructor or their body is empty, and at executor start when a
parameter fixes the constructor. A source typed `Key identities.OAuth`
never plans an `EmailPass` arm. Plans are specialised per pattern of
present `Option` parameters ([queries.md](queries.md)).

### 8.4 Joins

Joining a matched sum with another input either distributes the join
into every arm, costing each on its own heap, or hoists it above the
union when the join key is a field every arm outputs. The optimizer
chooses per query. When the join is distributed, the other side is
built once into a hash table that every arm's pipeline probes.

```
from i in identities, u in users where i.user == key u
select { name: u.name, login: match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub }

build: Scan(users) -> HashBuild(id)
arm 0: Scan(EmailPass) -> Probe -> sink
arm 1: Scan(Google)    -> Probe -> sink     -- or IndexNL on users' key index
arm 2: Scan(Apple)     -> Probe -> sink
```

### 8.5 Statistics

Statistics are kept per constructor heap: the row count, and per field
an online reservoir sample and a mergeable distinct-count sketch.
Statistics of a field over the whole sum are the merge of its heaps'
sketches. A match's cardinality estimate is the sum of its arms'
estimates, each from its own heap, since arms are disjoint.

### 8.6 Plans

A registered query is planned and checked when it is registered, and
its plan is stored in the catalog with the schema version and the
statistics it was costed with; calls never plan. A plan names
constructor ids, so it stays valid while the constructor sets it reads
are unchanged. A plan is replanned when a sum it reads changes
version, and when statistics of a heap it reads drift past a threshold
(for example a constructor grown from empty). Plan pinning, regression
reverts and replanning on a new engine release are in
[catalog.md](catalog.md#7-plans).

Rulings: exec/flow/push, exec/model/adaptive, exec/parallel/morsel, exec/parallel/small-heaps/pack, exec/batch/arrow, exec/value-sum/dense, exec/value-sum/special/option-validity, exec/value-sum/special/enum-tag, exec/value-sum/special/one-ctor, exec/interleaved/sel-vectors, exec/probe/fuse, exec/prune/compile, exec/prune/init, exec/join/cost, exec/join/rebuild/shared-build, exec/stats/per-ctor, exec/stats-form/sketch, exec/arm-card/per-arm, exec/plan-cache/at-register, exec/replan/schema, exec/replan/stats

## 9. Replication

### 9.1 Physical replicas

One database is one node's serial history; it is never partitioned
across nodes. It scales by replicas and by many databases.

A replica receives the redo log itself and runs recovery continuously.
Its pages, TIDs, xids and constructor ids are identical to the
primary's, so the primary's safe snapshot markers mean the same on it,
and a page quarantined on one copy can be restored from the other by
an operator.

### 9.2 Reads on a replica

A replica's read-only transactions run only at safe snapshot
markers (§7.4): at the latest marker the replica has applied, or by
waiting for the next. They take no SIREAD locks and never abort with a
serialization failure.

The primary emits safe points periodically: when no marker has been
logged for the setting `SafePointDelay` (default 100 ms), it delays the
start of new read-write transactions until the running ones finish and
a marker forms, by at most `SafePointDelay`. While a read-write
transaction runs without bound, no marker may form at all. Each
replica read reports the commit position and the age of its snapshot.

A commit returns its log position, a `Position = { epoch: I64, lsn:
I64 }` whose values fit the wire's u32 epoch and u64 LSN. A replica
read given a position waits until the replica has applied it and a
safe snapshot covers it, for at most the setting `ReplicaWait`
(default 5 s); past that it fails with `Retry { reason: SafePointTimeout {..} }`
([transactions.md](transactions.md#8-commit-positions-and-replica-reads)).
A position not on the history the node serves is refused with
`Rejected { reason: PositionNotOnHistory {..} }`.

### 9.3 Vacuum feedback

Each replica reports to the primary, per heap, the older of the oldest
snapshot its readers hold and the latest safe snapshot marker it has
applied, so a replica with no readers still reports a bound. The
primary folds these into each heap's watermark (§7.3). Replay that
would remove a version a replica reader can see waits for that reader
up to the setting `ReplicaVacuumWait`, then cancels the reader with
`Retry { reason: ReplayConflict {..} }`. A registered query on a replica holds back only the heaps it
reads.

### 9.4 Schema, leases and upgrades at replicas

Schema operators and stage transitions travel in the log (§3.1), so a
replica applies a constructor's declaration before any of its rows.
A client connecting at a replica renews its bundle's lease, which is a
catalog write; the replica forwards the renewal to the primary,
retrying until it is acknowledged. Renewals are idempotent, so a
renewal applied twice extends the lease once
([applications.md](applications.md#42-leases)).

A lease's expiry is computed from the last renewal record in the log,
on the primary and on replicas alike. A replica refuses binds and calls
of the bundle with `Rejected { reason: BundleExpired {..} }` from its
expiry. The primary treats the lease as expired only after the further
setting `LeaseGrace` (default 1h), which covers clock skew between
nodes, and from then refuses binds and calls of the bundle too. A
renewal that arrives after expiry is refused with `BundleExpired`; the
bundle must be registered again.

Replicas upgrade to a new engine release before the primary; the primary writes a new format
only after every replica reports the new release.

### 9.5 Commit acknowledgement

Each transaction chooses its durability level
([transactions.md](transactions.md#7-durability-levels)):

| Level | Acknowledged when | Lost on failover |
| --- | --- | --- |
| local | durable in the primary's log | commits past the promoted replica's applied position |
| quorum | durable in the logs of a quorum of nodes | none, while a quorum survives |

Promotion reports the position it starts from, which is the loss
window for local commits.

### 9.6 Failover

Promotion is manual. Promoting a replica first fences the old primary
from its storage or network, then starts a new epoch: the file header
and every log block the new primary writes carry it. A node that sees a
log block or a peer with a higher epoch stops writing. A position from
an old epoch past the fork point is refused with `Rejected { reason:
PositionNotOnHistory {..} }`, and a commit reaching the fenced old primary
fails with `Fenced`. The old primary rejoins only after its pages are resynced to
the new history.

### 9.7 Embedded replicas and log shipping

The embedded library ships its log segments and periodic base copies
to object storage, which is a continuous backup (§9.9). An application
may also hold an embedded replica of a server primary: reads run
locally at safe snapshots; every read-write transaction is forwarded
to the primary and runs there. The replica forwards each with a client
id and request numbers and resends it until it learns the outcome, so
a lost acknowledgement neither loses nor repeats a commit. Ad hoc
writes forward as `CallText`, and a load into one relation forwards as
`Append`; a load into several relations is refused at the replica with
`Rejected { reason: ReadOnlyReplica {..} }`. A write call at a server replica, which
does not forward, is refused with `Rejected { reason: ReadOnlyReplica { primary
} }` ([wire.md](wire.md)).

### 9.8 Change streams

External consumers are feeds, declared `feed NAME of r.S` in a bundle
and registered with it, pinned to its schema version. Under
readers-first a feed blocks `enable r.C` only when it names `r` whole
([evolution.md](evolution.md#8-readers-first)). A feed reads with
the bundle's definer rights: the bundle's owner holds `read` on the
set, and subscribing needs the `call` grant on the bundle or the feed
([wire.md](wire.md#9-feeds)). Each subscriber receives the committed change sets of the
feed's constructor relations, in commit order. A retag appears as a delete from the old constructor relation
and an insert into the new one in the same change set. Retention is
bounded; a consumer whose position falls behind it restarts from a
fresh snapshot and the position after it ([wire.md](wire.md#9-feeds)).

A transaction that writes a constructor relation of at least one
registered feed, a bulk load included (§3.7), writes logical change records to the redo log for
those relations: the inserted and deleted rows per constructor, ahead
of its commit record. Relations of no registered feed get no logical
records, so a feed's history starts at its registration: a subscription
from an earlier position is refused with `HistoryUnavailable`. Recovery and physical replicas ignore logical
records; the change stream is read from them.

### 9.9 Backups and point-in-time restore

A backup is a page copy of the data file taken while writers run, plus
the log from the redo LSN of a checkpoint taken at the copy's start to
its end position. Full-page images are logged throughout, whatever the
device guarantees (§3.1), so replay from that redo LSN repairs torn and
inconsistent pages. An
incremental backup copies only pages whose LSN exceeds the previous
backup's start. Backups copy pages and log as stored, encrypted or not
([security.md](security.md#9-encryption-at-rest)).

Every backup carries a manifest of page and segment checksums, checked
without restoring, and is proven by a test restore: open, recover, then
run `verify` with every structural, cross and logical check over the
result ([catalog.md](catalog.md#111-verification)).

With a continuous log archive, a restore replays a base to a log
position, to a wall-clock time through the commit time map, or to just
before a schema operator:

```
$ adt restore s3://b/app restored.adt --before-version 12
$ adt restore s3://b/app restored.adt --to-time 2026-10-24T13:02:00Z
```

Rulings: repl/distribution/never, repl/unit/physical, repl/standby-reads/safe-snapshot, repl/standby-reads/lag/force-point, repl/standby-reads/lag/report-age, repl/session/bookmark, repl/unit/replica-vacuum/feedback, repl/schema/in-log, repl/schema/forward-leases, repl/upgrade/replicas-first, repl/commit-ack/local, repl/commit-ack/quorum, repl/commit-ack/rpo/stated, repl/failover/manual, repl/failover/split-brain/epoch, repl/failover/split-brain/fence, repl/embedded/ship-log, repl/embedded/embedded-replica, repl/embedded/local-writes/forward, repl/cdc/change-stream, repl/cdc/registered-consumer, repl/cdc/retag/ctor-pair, repl/cdc/retention/bounded, repl/cdc/retention/resnapshot, repl/backup/base-plus-log, repl/backup/incremental, repl/verify/manifest, repl/verify/restore-check, repl/pitr/log-archive, repl/pitr/op-log-point

## 10. Maintenance operations

### 10.1 Compaction

The file shrinks only by compaction, which copies every live version
into a new file:

- Offline, with the database closed. This is also how a setting fixed
  in the file format is changed.
- Online, while serving: each heap is scanned at one snapshot into the
  new file, indexes are built by merging sorted runs (§6.8), the log
  written since the snapshot is applied, and the database switches to
  the new file at a quiescent point. Both files exist for the
  duration.

```
$ adt compact app.adt --into app.compact.adt
```

Compaction gives versions new TIDs and pages new ids, so physical
replicas and incremental backups start from a new base afterwards.

### 10.2 Reindex

An index is rebuilt online or offline as described in §6.8.

### 10.3 Corruption scope

A constructor heap is the unit a detected corruption stops. When a
heap page fails verification, the heap is quarantined: queries that do
not read it continue, and compile-time pruning (§8.3) tells which do.
A write is refused where a constraint, reference or index maintenance
must read the quarantined heap. An index page holds entries of several
constructors, so a bad index page quarantines the whole index and
every plan that uses it. The error a call receives is in
[catalog.md](catalog.md#4-error-sums).

```
-- identities.Apple quarantined
from i in identities.(EmailPass | Google) select i.id    -- runs
from i in identities select i.id                         -- fails
```

A quarantined object is restored by reindexing (an index) or by
restoring its pages from a replica or a backup plus log (a heap).

Rulings: ops/compact/offline-copy, ops/compact/online-copy, ops/reindex/online, ops/reindex/offline, ops/halt-scope/heap
