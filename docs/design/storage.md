# Storage

The pager, the redo log and recovery provide the file, its pages and
the log to the heaps and versions of [mvcc.md](mvcc.md), the indexes of
[indexes.md](indexes.md), replication and backup
([replication.md](replication.md)) and `verify`
([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)).

## 1. The file

### 1.1 One page space

A database is one data file plus its redo log as segment files in a
sibling directory (§5.3). Every heap, index, free space map,
visibility map, commit log page and catalog page is a set of pages in
the data file.

Invariants:

- Page ids form one dense space over the whole file. The buffer pool
  and every TID use that space, so a page id names one page whatever
  heap or index holds it.
- Every page of the file, data and index alike, has the page size
  recorded in the file header. The page size is a power of two of at
  most 64 KiB and is fixed when the file is created.

Adding a constructor gives its heap an empty set of pages; removing one
frees its pages once its heap is dropped
([mvcc.md §10.4](mvcc.md#104-removing-a-constructor)).

Free pages are tracked by an allocation bitmap of one bit per page,
held in bitmap pages of the file. Freed pages are reused for later
allocations. Each bitmap page covers a group of `(page size - 24) * 8`
consecutive page ids; group `g` has its bitmap page at
`max(g * group size, 2)`, inside the group it covers, and its own bit
is set. Page 2 is therefore the first bitmap page.

### 1.2 File header

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
format upgrade ([../spec/catalog.md §12.2](../spec/catalog.md#122-upgrades-and-downgrade)),
the location of the last checkpoint record (§6), the root of the
catalog, the encryption parameters when the file is encrypted (§4),
the page size (§1.1),
whether page writes are atomic on the device (§5.2), and the current
primary epoch ([replication.md §6](replication.md#6-epochs-and-fencing)).

A header copy occupies the first 4096 bytes of its page; copy 1
starts at the page size, so a reader that cannot read copy 0 finds
copy 1 by trying each allowed page size and keeping the copy whose
page size field names its own offset. The rest of both pages is
unused.

```text
offset  size  field
0       8     magic "adtbfile"
8       4     crc32c over the 4096 bytes with this field zeroed, seeded with the copy number (0 or 1)
12      4     layout version of the copy, 1
16      8     sequence number
24      16    file id
40      4     page size
44      4     log block size
48      8     log segment size
56      2     lowest page format present
58      2     highest page format present
60      2     format upgrade finalization state
62      2     flags: bit 0 page writes are atomic on the device
64      8     LSN of the last checkpoint record
72      8     page id of the catalog root, 0 before the catalog exists
80      8     page id of the quarantine marks (§8), 0 before the page exists
88      4     current primary epoch
92      4     encryption scheme, 0 for a plaintext file
96            encryption parameters; zero in a plaintext file
```

A seed here, and for page checksums, is fed to the CRC-32C first as
eight little-endian bytes.

Rulings: layout/files/single-file, buffer/cache/vm-reserve/global-pid, layout/alloc/bitmap, layout/header/version

## 2. Pages

### 2.1 Page header

Every page starts with a header. A plaintext page has:

```text
offset  size  field
0       8     lsn       LSN of the last log record applied to the page
8       4     checksum  CRC-32C over the page with this field zeroed, seeded with the page id
12      2     format    format version of the page's contents
14      2     kind      page kind, below
16      4     owner     heap id of a heap page, index id of an index page, 0 otherwise
20      2     flags     defined by the kind: all-visible (heap), half-dead and incomplete split (index), ...
22      2     reserved, zero
24            kind-specific contents
```

| Kind | Value |
| --- | --- |
| heap fixed-slot | 1 |
| heap slotted | 2 |
| index leaf | 3 |
| index inner | 4 |
| hash bucket | 5 |
| free space map | 6 |
| visibility map | 7 |
| allocation bitmap | 8 |
| commit log | 9 |
| catalog | 10 |
| quarantine marks | 11 |

A page whose bytes are all zero was never written: it reads as zeros
with LSN 0, and verifies.

An encrypted page replaces the checksum by an authentication tag and
adds a nonce and a data key version (§4.1).

The kind-specific contents start at offset 24 in every plaintext page.
A heap or index layout may rely on that offset and on the page LSN,
the checksum and the format version being where the table puts them.

The pager owns bytes 0 to 11, the LSN and the checksum; the layout
that owns the page kind owns bytes 12 to the end, including format,
kind, owner and flags.

### 2.2 Format versions

Rows and pages are little-endian. Index keys use their own
order-preserving encoding
([indexes.md §3](indexes.md#3-key-encoding)), independent of byte order.

Each page states its own format version, so one heap may hold pages of
several row formats at once while each page stays uniform.

Postconditions for a layout that owns a page kind:

- A page written in an old format is decoded by the old decoder and is
  rewritten in the newest format only when it is next written.
- The file header records the range of formats present, so a decoder
  can be retired once no page of its format remains.

Rulings: layout/checksum/in-page-32, layout/endian/little, layout/format-version/read-old, layout/heap-format/page-version

### 2.3 Page access

Heaps, indexes and every other page layout reach pages only through
the pager, `adtb::storage::Pager`. Every operation takes the pager by
`&self`, and any number of threads share it; the latches behind this
section's calls are §2.4.

TODO: an unlogged change of a clean frame, for kill bits (§5.2), is undecided.

```text
pager.read(pid) -> Result<PageShared, PageError>     // a shared latch, read as &[u8]
pager.update(&[pid, ...], |pages: &mut [&mut [u8]]| ...) -> Result<Lsn, PageError>
pager.change(&[(pid, vec![Edit { offset, bytes }]), ...]) -> Result<Lsn, PageError>
pager.allocate(format, kind, owner) -> Result<Pid, PageError>
pager.reserve() -> Reservation
reservation.page(format, kind, owner) -> Result<Pid, PageError>
reservation.image(pid) -> Vec<u8>                  // the page as allocation formats it
reservation.free(pid)                              // freed by the same update
pager.update_with(reservation, &[pid, ...], |pages: &mut [&mut [u8]]| ...) -> Result<Lsn, PageError>
pager.update_all_with(reservation, &mut [&mut Exclusive], |pages| ...) -> Result<Lsn, PageError>
pager.free(pid) -> Result<Lsn, PageError>
pager.allocated() -> Result<Vec<Pid>, PageError>
pager.commit(xid, &[pid, ...], |pages: &mut [&mut [u8]]| ...) -> Result<Lsn, PageError>
pager.abort(xid, &[pid, ...], |pages: &mut [&mut [u8]]| ...) -> Result<Lsn, PageError>
pager.reserve_xids(next) -> Lsn
pager.flush_log()
pager.durable() -> Lsn
pager.wait_durable(lsn) -> Position                // group commit; the record's commit position
pager.log_safe_snapshot(position, &snapshot) -> Lsn
pager.max_threads() -> usize
pager.catalog_root() -> Pid
pager.set_catalog_root(pid)
pager.wall() -> SystemTime
pager.close()
PageError { pid, damage: Checksum | Unreadable | Stale { expected, found } }
```

A page is latched, resident and unchanged by anyone else for the
duration of the call that names it. `read` returns the whole page,
header included, under a shared latch released when the guard drops.

Preconditions:

- `update`, `change`, `commit` and `abort` name each page once, and
  never pages 0 or 1.
- The pages latched at once, by every thread together, are fewer than
  the pool's frames. An update that allocates also latches one bitmap
  page per allocation bitmap group it allocates from.
- A thread latches at most five pages at once (`LATCHES_PER_THREAD`:
  a B-link operation's four and the bitmap page), and at most
  `max_threads()` threads latch at once, the pool's frames divided by
  five. The options state the threads; opening a pool too small for
  them panics. In debug builds and under the simulator, taking a sixth
  latch panics, naming the count; a latch the pager takes for a page
  it reads in or allocates counts.
- A thread asks for no latch on a page it holds a guard on, through any
  call: a call that would latch that page again waits forever. In
  debug builds and under the simulator the call panics instead, naming
  the page. An optimistic read beside a shared latch of the same
  thread is allowed.
- They write no byte below offset 12 and none past the page's end.
- `close` is called once every other call on the pager has returned,
  and no call follows it.
- `free` and `reservation.free` name an allocated page that nothing
  refers to once the update is applied.

Postconditions:

- `read`, `update` and `change` hand out no page that fails
  verification (§3); they fail with `PageError` instead, and the page
  is refused from then on. The caller quarantines the heap or index it
  maps the page to (§8).
- `update` and `change` change every page they name or, failing, none.
  `update` gives its closure a copy of every page and logs the bytes
  the closure changed; `change` applies the edits as given. Each page
  changed gets one log record and its LSN; the call returns the last.
  They take their latches in page id order.
- `commit` and `abort` change their pages as `update` does and log the
  commit or abort record of `xid` after the page records; they return
  the record's LSN.
- The records of one call are contiguous in the log and in one batch
  (§5.3), so after a crash all of them are replayed or none.
- A change is durable once `durable()` is past the LSN it returned,
  which `flush_log` ensures for everything logged before it.
- `reservation.page` holds a free page that no other reservation or
  allocation gets, and marks nothing; it fails when an allocation
  bitmap page it reads is damaged, and a free page found damaged is
  refused and skipped. Dropping a reservation unused gives its pages
  back, with nothing logged. A caller may hold the latch of a free
  page, one it found by a stale read, while it allocates or frees:
  `reservation.page` waits for a free page's latch only after other
  allocations can proceed.
- `update_with` and `update_all_with` take a reservation whose every
  page is among the pages they change. In the batch that holds the
  change, each reserved page is marked in the allocation bitmap and
  formatted with its format, kind and owner, flags zero and contents
  zero, before the closure sees it. Each page the reservation frees is
  cleared in the allocation bitmap in the same batch; its contents stay
  until it is allocated again. A crash keeps the allocation or the
  freeing and the change that links or unlinks it, or neither.
- `allocate` is a reservation of one page allocated by an update that
  changes nothing else, and `free` one freed the same way. An
  allocation is not undone when the transaction that made it aborts.
- `allocated` returns, in order, every page the allocation bitmap marks
  allocated, leaving out the pager's own: pages 0 and 1, the bitmap
  pages and the quarantine marks' page (§8). It fails
  when an allocation bitmap page is damaged.
- A failed write or force, including one that grows the file, crashes
  the process ([README.md §4](README.md#4-errors)); no call returns it.
- `wait_durable(lsn)` returns once the record at `lsn` is durable, with
  its commit position: the primary epoch and `lsn`. A commit is
  acknowledged at durability level local only after it returns.
  Records appended by other threads while a batch is forced are forced
  together in the next batch, by one of the waiters.
- A record appended after `wait_durable` returned for another has a
  later position, so a commit acknowledged before a transaction began
  has an earlier position than that transaction's commit.
- `log_safe_snapshot` logs a safe snapshot marker: the snapshot as the
  transactions layer encodes it, and the commit position up to which
  it covers commits. Markers are durable like any record; recovery
  ignores them and `verify` lists them.
- `catalog_root` returns the page the file header names as the
  catalog's root (§1.2), `Pid(0)` before one is set.
  `set_catalog_root(pid)` forces the log, then writes and forces a
  header copy naming `pid`; a crash keeps the old root or the new
  one, and the new one only with every record logged before the call.
- `wall` returns the wall-clock time of the runtime the pager was
  created or opened with, the clock commit records carry.
- `close` makes every record logged before it durable, writes every
  changed page and forces the data file, takes a checkpoint, then
  closes the data file and every log segment the pager opened. An open
  after it replays no page record.
- A `create` or `open` that fails leaves no file of the database open.
- When every frame stays latched past the options' stall bound, the
  page-in waiting for a frame panics, reporting a stalled pool.
- Write-ahead, full-page images and checkpoints (§5.2, §6) are the
  pager's: a layout never writes a page to the file, forces anything
  but the log, or logs a page record itself.

```text
let pid = pager.allocate(1, PageKind::HeapSlotted, heap_id)?;
pager.update(&[pid], |p| slotted::insert(p[0], &row))?;
let commit = pager.commit(xid, &[status_page], |p| set_status(p[0], i, Status::Committed))?;
let position = pager.wait_durable(commit);   // now acknowledge the commit, at `position`
```

### 2.4 Latches

A consumer that holds a page across several steps reaches it through
one of three guards, each over the page state word of
[sys.md §3](sys.md#3-page-state-word):

```text
pager.shared(pid)     -> Result<Shared, PageError>      // count 1..252
pager.exclusive(pid)  -> Result<Exclusive, PageError>   // 253
pager.optimistic(pid) -> Result<Optimistic, PageError>  // version, no latch
optimistic.read(|page: &[u8]| ...) -> T             // a copy taken through atomic loads
optimistic.validate() -> bool
optimistic.upgrade() -> Option<Exclusive>           // only if the version is unchanged
exclusive.update(|page: &mut [u8]| ...) -> Lsn      // diffed and logged as in §2.3
exclusive.change(edits) -> Lsn
pager.update_all(&mut [&mut Exclusive], |pages| ...) -> Lsn // several pages in one batch
pager.commit_all(xid, &mut [&mut Exclusive], |pages| ...) -> Lsn // and the commit record
```

Invariants:

- A page under a shared or exclusive latch is resident and is not
  evicted or written by the pager until the latch is released.
- Only an exclusive latch changes a page. Releasing it after a change
  increments the version; so does eviction.
- The record of a change is appended while its page's exclusive latch
  is held, so the LSN order of one page's records is the order of its
  changes.
- The pager holds a page exclusive while it reads it in or writes it
  out ([sys.md §3](sys.md#3-page-state-word)), and takes no latch on a
  consumer's behalf except by try-lock: eviction skips a latched page.

Preconditions:

- A consumer that holds latches on several pages acquires them in an
  order it defines for its structure, so no cycle of waits forms; a
  B-link tree takes them top-down and left to right. The pager adds no
  order of its own.
- A waiting latch spins only through `yield_now` or `park`
  ([runtime.md §1](runtime.md#1-time-threads-and-randomness)).
- What an optimistic read copied is not trusted until `validate`
  returns true: every length or offset it reads is bounds-checked
  before use ([sys.md §2](sys.md#2-unsafe-invariants)), and a failed
  validation restarts the read.

Postconditions:

- An optimistic read that validates saw the page as one exclusive
  holder left it, with no change between the version read and the
  validation.
- `upgrade` succeeds only when no exclusive latch was taken on the page
  since the optimistic guard was made.
- The postconditions of §2.3 hold for each guard's calls: verification
  before a page is handed out, all-or-nothing multi-page changes in one
  batch, durability once `durable()` passes the returned LSN.

```text
// optimistic lock coupling, one level of a B-link descent
loop {
  let node = pager.optimistic(pid)?;
  let child = node.read(|p| blink::child_for(p, &key));
  if !node.validate() { continue; }
  pid = child?;
  break;
}
```

Rulings: buffer/cache/vmcache, btree/latch/racy/atomic-reads, btree/structure/blink

## 3. Corruption detection

Every page read from the file is verified before the pager hands it
out. A page that fails verification is reported to the caller as an
operating error and never handed out as data.

| Fault | Detected by |
| --- | --- |
| Bit rot, latent sector error | Checksum (or tag) mismatch on read |
| Misdirected write | The page id seeds the checksum; a page read from the wrong place fails it |
| Torn write | Checksum mismatch; repaired from a full-page image in the log (§5.2) |
| Lost write | Page LSN older than the retained log says it must be, in recovery (§7) and in `verify` and scrub ([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)); a page read back after the pool dropped it, older than it was then, while the pool's bounded table of dropped pages still holds it (§10.3) |

A page that fails verification on read quarantines the heap or index
holding it (§8).

Outside what the engine detects: a device that drops a forced write of
the last batch of the log before a crash, or damages a block of that
batch, leaves the log ending at the batch before. Its commits were
acknowledged and are lost without a report, since recovery cannot tell
the batch from one that was never forced (§7). Earlier in the log the
same damage is a gap and stops recovery.

Rulings: buffer/faults/corrupt-detect, buffer/faults/stale-page/lsn-check

## 4. Encryption sealing

### 4.1 Pages

When a database is encrypted, the pager encrypts every page as it is
written and decrypts it as it is read. Every consumer of the pager sees
plaintext pages only; ciphertext exists only in the file. The cipher is
XChaCha20-Poly1305:

```text
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
  already dirty may be written again with kill bits
  ([indexes.md §8](indexes.md#8-notes-for-the-implementation)) set
  since its last write and no new LSN. Kill bits set on a clean frame
  log a full-page image first (§5.2).
- The associated data binds the file id (from the file header), the
  page number, the LSN and the data key version, with the other clear
  fields. A page moved to another address or another file, or with its
  LSN altered, fails the tag.
- A page replaced by an older valid ciphertext of itself carries an
  older LSN under a valid tag. Recovery detects it by the LSN check
  against the log (§7); outside recovery, detecting such a rollback
  is out of scope.
- The tag replaces the CRC-32C checksum. A tag that fails is a
  corrupt or forged page and is handled as any corruption (§3).

Data keys are wrapped by a key-encryption key and named by version;
rotating the data key makes new writes use the new version, and older
pages move to it as they are rewritten. Key sources, rotation and the
treatment of backups are in
[../spec/security.md §9](../spec/security.md#9-encryption-at-rest).

### 4.2 Log blocks

In an encrypted database each log block is sealed as a page is:

```text
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

Rulings: security/encrypt/page, security/nonce/random, security/integrity/aead

## 5. The log

### 5.1 Records

The log is redo-only. Old row versions stay in their heaps
([mvcc.md](mvcc.md)), so an aborted transaction's versions are invisible
by the commit log and need no undo; vacuum removes them later. A record
describes a change to one page, or a transaction or system event:

| Record | Contents |
| --- | --- |
| page change | page id, the page's LSN before the change, the change |
| full-page image | page id, the page's LSN before the change, the whole page after it |
| commit | xid, wall-clock time of commit; the record's position is the commit position |
| abort | xid |
| xid allocation | the next xid reserved ([mvcc.md §4](mvcc.md#4-transaction-ids-the-commit-log-and-snapshots)) |
| schema operator | an operator of a migration, and the schema version the migration produces ([schema-change.md §3](schema-change.md#3-log-records)) |
| stage transition | an online schema state change ([schema-change.md §2](schema-change.md#2-staged-states)) |
| safe snapshot marker | a snapshot no read-write transaction can make unsafe, as the transactions layer encodes it, and the commit position it covers ([replication.md §2](replication.md#2-safe-snapshot-markers)) |
| logical change | inserted and deleted rows of one constructor relation that has a registered consumer ([replication.md §7](replication.md#7-logical-change-records)) |
| checkpoint | redo LSN and dirty page table (§6) |

A retag, a split or any multi-page change is several page records in
one transaction. A caller may rely on it being atomic: its versions
become visible only through its commit record.

### 5.2 Full-page images

Invariant: the change that dirties a clean frame, the change that sets
the frame's `rec_lsn` (§6), logs a full image of the page instead of a
page change record. Every write of a page to the file therefore
writes a state whose last full image lies at or after the page's
`rec_lsn`, and so at or after the redo LSN of any checkpoint taken
while the page was dirty. Recovery restores a torn page from that
image.

A consumer that dirties a page by a change it does not log as a
record (kill bits, [indexes.md §8](indexes.md#8-notes-for-the-implementation))
relies on this rule: on a clean frame the change logs a full-page
image first.

When the device and file system guarantee atomic writes of a whole
page, no full-page images are logged outside a backup: a page write
either lands whole or not at all, so no page can be torn. The engine
detects the guarantee when it opens the file (for example from the
atomic write unit `statx` reports for direct I/O) and records it in the
file header; a file opened where the guarantee does not hold logs
images again from its next change of each clean frame. While a backup
runs ([replication.md §9](replication.md#9-backups)), full-page images
are logged whatever the device guarantees, since the backup's page copy
can be torn.

### 5.3 Blocks and segments

The log is a sequence of preallocated segment files in the database's
log directory, `storage::Options::log_dir()` of the options the pager
opens with, named by the LSN of their first byte. The data file and
that directory are every file a database has. The log is written
with direct I/O in whole blocks of the device's logical block size.
Each group commit (§10) is one batch of whole blocks, padded after its
last record.

Invariants:

- A block that has been acknowledged is never written again.
- Batches are numbered by a sequence that increases by one per batch.
- The blocks of a batch are written and forced together, and the next
  batch is written only after that force has completed.
- A commit is acknowledged only after its whole batch is forced.

```text
log block
  0   u64  lsn of the block's first byte
  8   u64  batch sequence number
  16  u32  number of blocks in the batch
  20  u32  index of this block within the batch, from 0
  24  u32  epoch of the primary that wrote it (replication.md §6)
  28  u32  crc32c over the whole block with this field zeroed
  32  u16  bytes used
  34  u16  reserved
  36       records; a record may continue into the next block
```

A record never continues past the last block of its batch. A segment's
name is the LSN of its first byte as sixteen lowercase hexadecimal
digits; its size, recorded in the file header, is a multiple of the
block size, so a block never spans two segments.

Log segments are preallocated and zero-filled before use, so a write
inside a segment changes no file size, and the force at commit is
`fdatasync`. Creating or recycling a segment file is followed by an
`fsync` of the directory.

Rulings: engine/redo-mvcc, layout/torn/fpw, buffer/io-path/log-align/slot-pad, buffer/force/fdatasync

## 6. Write-ahead and checkpoints

Invariants:

- A dirty frame is written only after the log is durable to the
  frame's page LSN.
- A written frame is counted clean, and drops out of the dirty page
  table, only after an `fdatasync` of the data file covers the write.

Checkpoints are fuzzy and never stop writers. Each dirty frame records
the LSN that first dirtied it (`rec_lsn`). A checkpoint record holds
the dirty page table and the redo LSN, the minimum `rec_lsn` over it;
the file header is updated to point at the record.

```text
checkpoint record
  u64 redo_lsn = min(rec_lsn)
  u32 n
  (u64 page_id, u64 rec_lsn) * n
```

Log before the redo LSN is no longer needed for recovery. Its segments
may be recycled only once replication, archiving and change consumers
no longer need them ([replication.md §1](replication.md#1-the-log-stream)).

Rulings: buffer/checkpoint/fuzzy

## 7. Recovery

Recovery reads the checkpoint record named by the file header and
replays the log from its redo LSN to the end of the log. For each
record naming page `p`:

| Condition | Action |
| --- | --- |
| Full-page image | Install the image; the page LSN becomes the record's LSN |
| `p.lsn >= record.lsn` | Skip; the change is already on the page |
| `p.lsn == record.prev_lsn` | Apply the change; the page LSN becomes the record's LSN |
| `p.lsn < record.prev_lsn` | Lost write: quarantine the page's heap or index (§8) |

Replay verifies a page when it first reads it. A page that fails
verification is not corruption if a full-page image of it follows in
the replayed log: replay skips the page's records until that image and
installs it. A page with no such image quarantines its heap or index
(§8).

Each page record names the page's LSN before it. Replay reads the log
from the start of the batch holding the redo LSN, and a record whose
previous LSN is not the LSN of the record before it for the same page
marks a page that was changed from a stale copy: a lost write, which
quarantines the page's heap or index. `verify` checks the same chain
over the whole retained log, and that each page's LSN is that of its
last record there.

The end of the log is found by batch. A batch is accepted when all of
its blocks are present, each verifies, and they agree: the same
sequence number, block count and epoch, indexes 0 to count minus 1,
and LSNs that continue one another and the previous batch. Its
sequence number must be one more than the previous batch's. Replay
stops at the first batch that is not accepted. That batch and every
block after it are discarded: none of their commits was acknowledged
(§5.3). Before the first new write, the log tail after the last
accepted batch is zeroed and forced, so blocks of a discarded batch can
never be read as a continuation later. A later segment holding none
of the accepted log is removed.

Past the last accepted batch, with sequence number `n`:

| Block | Meaning |
| --- | --- |
| Zero | Never written |
| Valid, sequence `n + 1` | Part of the discarded batch |
| Valid, sequence `n` or lower | A stale block: reported as log damage |
| Neither zero nor valid where it lies | A torn block of the discarded batch, or damage; not reported |
| Valid, sequence `n + 2` or higher | Batch `n + 1` was forced, so the log is corrupt; recovery stops |

An unreadable log block in the range recovery reads also stops it.

Recovery ends with a checkpoint: it writes and forces every page it
replayed, then logs a checkpoint with an empty dirty page table and
points the file header at it.

Recovery reads both log and pages with direct I/O, so it reads what
is on the device and not a stale kernel cache.

Postcondition: after recovery the file holds every commit whose batch
was accepted, and the log tail past it is zeroed. Schema operators and
stage transitions are applied in log order with the data
([schema-change.md §3](schema-change.md#3-log-records)). Logical change
records are ignored ([replication.md §7](replication.md#7-logical-change-records)).

Rulings: engine/redo-mvcc

## 8. Quarantine

A constructor heap is the unit a detected corruption stops. When a
heap page fails verification, the heap is quarantined. An index page
holds entries of several constructors, so a bad index page quarantines
the whole index.

A commit log page that fails verification quarantines the whole
database, since the visibility of every relation reads the commit log:
every transaction is refused with the corruption fault, and the
database opens for `verify` and repair only
([mvcc.md §4.2](mvcc.md#42-the-commit-log)).

Quarantine is a mark the storage layer sets on one heap, one index or
the whole database, and keeps until the object is repaired:

```text
pager.quarantine(Quarantine::Heap(id) | Index(id) | Database) -> Result<(), PageError>
pager.release(object) -> Result<(), PageError>    // once repaired
pager.is_quarantined(object) -> bool              // its own mark or the database's
pager.quarantined() -> BTreeSet<Quarantine>
```

The marks live on one page of the file, of kind quarantine marks,
whose id the file header records (§1.2); the database's creation
allocates it. A mark or release is a logged change of that page,
durable when the call returns, so the marks survive a crash and a
restart and are read back at open. A heap or index stays quarantined
across restarts until `release`. A marks page that fails verification
at open quarantines the database, and `quarantine` and `release` then
fail with its damage.

The pager reports a damaged page to its caller (§2.3); the caller maps
the page to its heap or index and marks that. The planner and the
transactions layer read the marks and rely on them as follows
([execution.md §6](execution.md#6-quarantine)):

- a call whose read set
  ([../spec/queries.md §12.4](../spec/queries.md#124-the-constructors-a-statement-reads))
  includes a quarantined heap fails, whether or not its plan prunes
  that heap;
- no plan uses a quarantined index;
- a write is refused where a constraint or reference check must read
  the quarantined heap, or where it changes a constructor relation a
  quarantined index covers.

The error a call receives is in
[../spec/catalog.md §4](../spec/catalog.md#4-error-sums).

```adt
-- identities.Apple quarantined
from i in identities.(EmailPass | Google) select i.id    -- runs
from i in identities select i.id                         -- fails
```

A quarantined object is restored by reindexing (an index) or by
restoring its pages from a replica or a backup plus log (a heap).

Rulings: ops/halt-scope/heap, spec/reads/static/static-types, spec/quarantine/index/cost-only

## 9. Bulk loads

A load fills new heap and index pages in private memory and logs each
filled page once as a full-page image, with no per-row records. The
images are what a physical replica replays, so replicas receive
the loaded rows. The load's change set is one transaction; its pages
become visible only through its commit record
([../spec/interop.md §4](../spec/interop.md#4-loading)). A load into a
constructor relation of a registered feed also writes logical change
records of the loaded rows
([replication.md §7](replication.md#7-logical-change-records)).

Rulings: interop/load/wal/page-images

## 10. Notes for the implementation

Non-binding.

### 10.1 Group commit

A committing transaction appends its commit record and waits until the
log is durable past it. The first waiter that finds no batch being
forced becomes the leader: under the flush lock it takes every record
appended so far as one batch, writes and forces it, and records it
durable. Appending takes only the log lock, so records keep arriving
while a batch is forced. Waiters spin through the runtime, checking
durability between attempts to lead; those the batch covered return
with it, and the rest take the next batch. Under the quorum durability level
([replication.md §5](replication.md#5-commit-acknowledgement)) the
leader also waits for replica acknowledgements of the batch, so one
round trip serves the batch.

Rulings: buffer/group-commit/leader

### 10.2 Flushing

Background flushing writes dirty frames continuously. When the log
written since the redo LSN nears its configured bound, the flusher
writes the frames with the oldest `rec_lsn` first, so a page dirtied
over and over cannot pin the log.

Rulings: buffer/checkpoint/log-pin/flush-oldest

### 10.3 Page cache

The buffer pool reserves anonymous virtual memory the size of the
page-id space; page `p` lives at `base + p * page_size`. A miss reads the
page into its own address; eviction writes it if dirty and releases
the memory with `madvise(MADV_DONTNEED)`. Index entries, chain links
and heap directories all name pages by page id, so any number of
references may name one page. The page state word and the pinning of
pages in flight are in [sys.md §3](sys.md#3-page-state-word).

Replacement is a clock over the resident pages: the hand marks an
unlatched page, and evicts it if it is still marked when the hand
comes back; any latch or optimistic read clears the mark.

The pool reserves the range for `max_pages` page ids when it opens.
The dirty page table, the set of pages changed since their last
write, and the set written since the last data force sit beside the
page memory under one lock. The pager remembers the LSN a page had
when it was evicted, to catch a lost write when the page is read back,
in a direct-mapped table of four slots per frame: a later eviction of
a page sharing the slot replaces the entry, and the page it displaced
is no longer checked until it is evicted again. Group commit is §10.1.
Pages in flight for a read are held exclusive; writes go out under a
shared latch. Recovery and `verify` read the log in chunks of 64 KiB,
holding one chunk and one batch at a time; recovery reads it twice,
once to find the end and once to replay.

Rulings: buffer/cache/vmcache, buffer/cache/vm-reserve/global-pid

### 10.4 Encryption staging

A write encrypts into a staging buffer that is then written; the
frame stays plaintext, so optimistic readers never see ciphertext.
A read decrypts in place in the frame before the frame is published.

Rulings: security/encrypt/io-cost/staging

### 10.5 Compaction

The file shrinks only by compaction, which copies every live version
into a new file:

- Offline, with the database closed. This is also how a setting fixed
  in the file format is changed, the page size among them.
- Online, while serving: each heap is scanned at one snapshot into the
  new file, indexes are built by merging sorted runs
  ([indexes.md §7](indexes.md#7-building-and-rebuilding)), the log
  written since the snapshot is applied, and the database switches to
  the new file at a quiescent point. Both files exist for the
  duration.

```sh
$ adtb compact app.adt --into app.compact.adt
```

Compaction gives versions new TIDs and pages new ids.

Rulings: ops/compact/offline-copy, ops/compact/online-copy, layout/files/shrink/reuse-only
