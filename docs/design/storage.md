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

A group starts in the batch that first allocates in it, which formats
its bitmap page. Groups start in order, so a file holding a page of
group `g` has started every group up to `g`.

Invariants:

- A started group's bitmap page is never blank. A bitmap page that reads
  blank in a started group is damage: it is refused, and no allocation or
  free reads it as free.
- A group is known started when the file header's bitmap group count
  covers it, or its bitmap page lies inside the file once recovery has
  written the pages the log holds.
- Every group the header counts has its starting batch forced to the
  log before the header copy is written, so a crash never leaves a count
  past a group that did not start.
- A header's checkpoint never lies past the batch that started a group
  the header does not count: each open's recovery counts the groups the
  file reaches into, once it has written what the log holds and before
  its checkpoint, and a running checkpoint that captures a lower count
  finds that group's bitmap page dirty.
- The stored count only rises.

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
the page of the damage list (§6) with the stale log blocks recovery
found and the list does not hold yet (§7), the page size (§1.1),
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
12      4     layout version of the copy, 3
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
88      8     current primary epoch
96      4     encryption scheme, 0 for a plaintext file
100           encryption parameters, to 1024; zero in a plaintext file
1024    8     page id of the damage list (§6), 0 before the page exists
1032    8     stale log blocks past the end of the log (§7) not yet in the damage list, m; at most 62; the top bit set when recovery found more
1040    16m   (u64 LSN of the block, u64 kind 1)
2048    4     ended epochs of the file's history, n; at most 127, a restore past that forgets the oldest
2052    16n   (u64 epoch, u64 log end where it ended), oldest first
4088    8     allocation bitmap groups started (§1.1) as of the last checkpoint
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
reservation.latch(pid) -> PageExclusive            // the reserved page, never read from the file
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
  bitmap page it reads is damaged. It never reads the free page, and
  takes one refused while free unless the device could not read it.
  `reservation.latch` of a page not resident gives it as
  `reservation.image` does, whatever the file holds, and the caller of
  `update_all_with` latches each reserved page that way. Dropping a
  reservation unused gives its pages back, with nothing logged. A
  caller may hold the latch of a free page, one it found by a stale
  read, while it allocates or frees.
- `update_with` and `update_all_with` take a reservation whose every
  page is among the pages they change. In the batch that holds the
  change, each reserved page is marked in the allocation bitmap and
  formatted with its format, kind and owner, flags zero and contents
  zero, before the closure sees it. Its record is a full-page image
  that continues no chain (§7), so nothing logged of the page before
  judges it. Each page the reservation frees is cleared in the
  allocation bitmap in the same batch, with a freed record; its
  contents stay until it is allocated again. A crash keeps the
  allocation or the freeing and the change that links or unlinks it, or neither.
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
- A resident page's bytes change only by a logged change under its
  exclusive latch, or by one of the writers in `storage/pager.rs`: a
  page-in, recovery's replay of an image or a change, a reservation's
  blank frame, an overwrite of a damaged page that the next logged
  change starts a chain from, and a settle's restore of the file's
  copy. No code outside `storage/pager.rs` holds a latch that can
  write.
- Releasing an exclusive latch after a change increments the version;
  so does eviction.
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
| Truncated file | After recovery, a bit of the allocation bitmap set past the end of the file refuses that page, and a bitmap page of a started group past the end of the file (§1.1) reads blank and is refused as below; at open, each bitmap page past the end of the file with an expected LSN is refused, whatever its group; a page a directory entry, index entry, version chain or out-of-line pointer names that reads all zero is refused on every read and write that reaches it through that name |
| Lost or zeroed bitmap | A blank bitmap page of a started group (§1.1) is refused on every allocation, free and read of the bitmap, before anything is written, and marks the database (§8) |
| Lost write | Page LSN older than the retained log says it must be, in recovery (§7) and in `verify` and scrub ([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)); in `verify`, scrub and backup, a page whose last record is recycled, older than the page LSN map holds for it (§6); on every read of a page from the file, an LSN below its expected LSN, refused as stale: its last record in the retained log, or else its entry in the page LSN map, or the LSN of the last record logged for it since the open. A blank page with an expected LSN is stale with LSN 0. The commit log alone reads a stale status page, settles it and writes it back ([mvcc.md §4.2](mvcc.md#42-the-commit-log)) |
| Lost or misdirected log write | A page read from the file at or past the end of the log recovery kept, or past the log's next LSN once the open has written the page, refused as stale on every read, write, replay and backup that reads it, and in `verify` and scrub; a page LSN, on the page or as a record's previous LSN, that no retained record wrote: past the page's last record in a log with no gap, or at or past the end of the log recovery kept in a gapless log retained from at or before that end; in `verify` and scrub, as an `Unlogged` log finding (§7) |

A page that fails verification on read quarantines the heap or index
holding it (§8).

Outside what the engine detects: a device that drops a forced write of
the last batch of the log before a crash, or damages a block of that
batch, leaves the log ending at the batch before. Its commits were
acknowledged and are lost without a report, since recovery cannot tell
the batch from one that was never forced (§7), unless a page that batch
changed reached the file: `verify` then reports the page's LSN as
unlogged. Earlier in the log the same damage is a gap and stops
recovery.

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
  24    u64  epoch                   in clear
  32    u32  data key version        in clear
  36    ...  ciphertext of bytes used and records
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
| damage | page id and its damage, other than a page the device cannot read (§8) |
| freed | page id of a page a batch frees |

A damage or freed record is no page record: it changes no page,
continues no page's chain of records and gives no page an expected LSN.

The records of a page from one allocation to its next free are one
incarnation. A full-page image whose previous LSN is unknown starts
one: every allocation logs it, and so does a change of a page
overwritten while damaged (§8). A freed record ends one.

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
images again from its next change of each clean frame.

### 5.3 Blocks and segments

The log is a sequence of preallocated segment files in the database's
log directory, `storage::Options::log_dir()` of the options the pager
opens with, named by the LSN of their first byte. The data file and
that directory are every file a database has. The log is written
with direct I/O in whole blocks of the device's logical block size.
Each group commit (§10) is one batch of whole blocks, padded after its
last record.

A create of a file builds it inside a reserved directory beside it, the
file's name with `.creating`. The create makes the directory, then writes
and forces a marker in it holding a fresh create id, which is also the
new file's id; the file and its log directory are named by the id and
made only after the marker is forced. Once the new database is open, the
create forces the log, renames the log directory into the parent under
its own name, forces both directories, renames the file to its own name,
which is the commit point, forces both again, and removes the marker and
the directory. A later create removes what the reserved directory holds
only when its marker is whole and every other entry is the file or the
log directory its id names, the log holding only segments, and on the
host only while it holds the lock on that file, taken without waiting;
it removes the marker last. When the log directory at its own name
begins with a block carrying the marker's id, the later create renames
the file into place instead. A reserved directory that is empty, or
holds only a marker with nothing written, is reused. Anything else
refuses the create and is left as it was, and a create never removes the
log directory at the file's own name.
An open without create of an empty file refuses when a log directory is
at its own name whose first block carries no id a whole marker names. A crash after the commit point
can leave the reserved directory holding only its marker. An open that finds the file whole
removes the marker and the directory when the marker is whole, names the file's id and is the
directory's only entry, on the host only when no symbolic link is at or in the directory, and
leaves anything else; a failure to remove them does not fail the open.

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
  24  u64  epoch of the primary that wrote it (replication.md §6)
  32  u32  crc32c over the whole block with this field zeroed
  36  u16  bytes used
  38  u16  reserved
  40  16   file id of the file this log belongs to
  56       records; a record may continue into the next block
```

Invariant: recovery replays a log only into the file it belongs to. A
restored file and its log keep the id of the file backed up. Open
answers damage when a valid block recovery reads holds another id. Creating a file claims its log
directory after creating the file and refuses a directory holding
anything else.

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
the LSN that first dirtied it (`rec_lsn`). Unless page writes are
atomic, a change to a frame written since its last change logs a full
image when the frame's latest image precedes the checkpoint the file
header names, so a frame logs at most one such image per checkpoint
interval. Once a force of the data file covers that write, the frame's
`rec_lsn` rises to where replay of the page may start: under atomic
page writes its first change since the write, else its latest image at
that change. Every later write of the frame then writes a state whose
last image lies at or after its `rec_lsn` (§5.2). A checkpoint record
holds the dirty page table and the redo LSN, the minimum `rec_lsn` over
it; the file header is updated to point at the record.

Recovery, a create, a backup, a recycle and a close each take a
checkpoint, and so does the checkpointer. From the end of an open's
recovery, or from the publication of a created file, the checkpointer
runs each time the log forced since the last checkpoint reaches the
threshold of `storage::Options::checkpoint_every`: it writes every
changed page, forces the data file and takes a checkpoint. Writers
never wait for it. The redo LSN of each checkpoint the checkpointer
takes is at or past the checkpoint before it, so recovery from the one
the file header names reads at most the log since the one before: the
threshold, what the cycle that took the named one logged, and what
followed it. A page refused while changed in memory is written before
its memory is dropped (§10.3), so it holds the redo LSN only until the
next force. A close stops the checkpointer before its
own checkpoint, waiting for a running checkpoint to end. Once the
open's readers of the log have run, or from the publication of a
created file, a cycle that finds a whole segment below the recycle's
floor (below) recycles, and the recycle's checkpoint is the cycle's; the
floor then comes from the checkpoint before it. Without a pin, an archive behind or
a refused page holding it, the log retained after a cycle starts at the
segment holding the redo LSN of the checkpoint before the one the
header names. The checkpointer never recycles while the database is
quarantined whole.

```text
checkpoint record
  u64 redo_lsn = min(rec_lsn)
  u32 n
  (u64 page_id, u64 rec_lsn) * n
```

Log before the redo LSN is no longer needed for recovery. A recycle
removes the segments that end at or before the start of the batch
holding the least of: the redo LSN of the last checkpoint the file
header durably names, the start of the current open's log until the
open's readers of the log have run, and the LSN every running backup or
reader of the log holds it from. A backup holds the log from the redo
LSN of its checkpoint, pinned with the record that logs it, until it
has copied the log. It removes
the oldest first and forces the directory after each, so the segments
present always run from the first of them with no hole. An open recycles once
every reader of the log it found has run: recovery, the settling of the
commit log, and the catalog's reads of a damaged commit log. With an
archive set, the open first waits for one archive pass, however long it
takes. An open that creates the file does not recycle. A close never
recycles. Replication and change consumers bound it too once they exist
([replication.md §1](replication.md#1-the-log-stream)).

With a log archive set, the archiver publishes each complete segment, one
whose every block is durable, under the file's id and epoch, in order and
after the epoch's file naming the epoch whose log this one continues, or
none. An epoch's first segment is the one holding the LSN where the file's
own log starts: 0 for a new file, including one loaded from a dump, and the
backup's end for a file restored from a backup, whose log continues the
backup's epoch. It archives only segments from there. Its
`archived_through()` is the end of the gapless run of segments published
there from that first segment, each matching the log's bytes, and no further
than the log's durable end. It is set only after the directory holding the
last of them is forced; it is that first segment's base when the
destination holds none, and `None` until its first pass since open has
listed the destination, or when the published run does not start at the
first segment.

Invariant: with an archive set, a recycler keeps every segment while
`archived_through()` is `None` or below the segment's end.

Before removing segments, a recycle folds the last LSN of each page
their batches name into the page LSN map, then takes a checkpoint. A
segment it cannot read whole is kept. The map is a radix tree of pages
of kind `LsnMap`: a leaf holds one LSN per page of the file, and an
interior page holds each child's page id and the least LSN that child
may have. The checkpoint record carries the root's page id, LSN and
height; a checkpoint record without them means no map. The root's LSN
there is that of the root's last change logged before the checkpoint
record. A fold that
makes a new root logs a record naming it in the same batch, and
recovery adopts the later of that record and the checkpoint's root. A
fold covers only pages below the file's end and grows the tree by a
level above the root when the file outgrows it. A map page that fails
its checksum, is no map page, or has an LSN below the one its parent,
or for the root the checkpoint, gives it quarantines the whole database
(§8). One the device cannot read is refused unmarked, as any page is. A
read of the map that meets either fails, and so does the listing of
allocated pages.

Postconditions of a recycle:

- Recovery from the checkpoint the file header names reads only segments
  it kept.
- No segment a running backup copies, or a reader of the log reads, is
  removed or closed.
- The map holds, for every page below the file's end named by a removed
  segment, at least the LSN of its last record there.

The expected LSNs a pager holds (§10.3) take 8 bytes of memory per page
of the file, plus an entry per page the retained log names past its
map entry.

Every page the pager refuses as damaged, other than one the device
cannot read, is recorded: the detection appends a damage record (§5.1)
holding only the log lock, and forces it before it returns. The damage
of a page whose last freed record follows its last allocation (§5.1),
in the log the open replayed or wrote, is held in memory instead: it is mapped in that open as a record is, never
logged, and the page's next allocation drops it. Recovery's
detections are recorded once it ends (§7), and a file quarantined whole
records in memory only (§8). A commit log status page settled from the
log is recorded as settled before its rewrite is logged. A checkpoint
folds every record into the damage list, one page of kind damage list the file header names
(§1.2): it takes the records under the log lock with the checkpoint
record, and logs the list page's change after that record, so recovery
from the checkpoint finds in the log every record the list lacks. A
checkpoint whose records have not changed since the last fold leaves
the page alone. An entry is 24 bytes: the page id with the damage's kind in its top byte,
then two LSNs. A full-page image of the page logged after its record
removes its entry, unless it is settled; the first record of an
incarnation (§5.1) and a batch that frees the page remove it of any
kind. Nothing else removes one: a
page repaired outside the engine, such as by copying its bytes back,
stays reported until an image or a free. Past the page's capacity the
list records the whole file as damaged and lists no more pages. A list
page that fails its checksum or is no list page quarantines the
database (§8). One the device cannot read quarantines it for the open
only, and the open's records then live in memory only. On a file
quarantined whole a checkpoint folds only the records the log holds
(§8).

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

The retained log runs from the first segment present. Segments missing
before it are recycled, not a gap; a missing segment between two
present ones is a gap. A lost write whose last record lies in a recycled
segment is not detected, and `verify` reports an `Orphan` finding only
while the log is retained from its start.

Under write-ahead logging no page carries an LSN the durable log does
not hold for it. `verify` reports an `Unlogged` log finding naming the
page, the LSN it carries and the LSN it is past:

- the end of the log recovery kept, for a page at or past that end and
  past its last record, when the retained log runs from at or before
  that end with no gap;
- otherwise its last record, for a page, or a record's previous LSN,
  past the page's last record in a log with no gap.

A page below that end with no record in a log retained from its start
is an `Orphan` finding. A check while writers run takes a page this open
wrote, at or past the log's end when the check starts, as written since.
A page the check reads from the file at or past the end of the log
recovery kept, or at or past the log's next LSN once the open has written
the page, is stale, and its `Unlogged` finding names that LSN.
Relation-scoped `verify` shows the `Unlogged` findings of the relation's
pages. Recovery
refuses a page it reads at or past the end of the log it kept, unless a
full-page image of it follows. A page it does not redo is refused by the
first read, write or `verify` that reaches it.

A full-page image with an unknown previous LSN, the first record of
an incarnation (§5.1), continues no chain and clears the page's damage.
`verify` reports a chain break of a page the allocation bitmap calls
free while the retained log holds it, unless a settle of the page
logged at those LSNs excuses it.

The end of the log is found by batch. A batch is accepted when all of
its blocks are present, each verifies, and they agree: the file's id,
the same sequence number, block count and epoch, indexes 0 to count minus 1,
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

Before it zeroes the tail, recovery adds the stale blocks it found to
those the file header holds, the 62 lowest, sets the header's overflow
bit when there were more, and forces the header. The first checkpoint
that folds the damage list folds them into it, an overflow as the list's
whole-file damage, and clears the header's. Recovery reads the damage
and freed records and the full-page images of the log past the
checkpoint it starts from onto the list, in log order. The pages it refuses are recorded once
the tail is zeroed, in the batch of its closing checkpoint; a crash
before that batch is durable leaves the page and the retained log as
they were, so the next open finds them again.

Recovery ends with a checkpoint: it writes and forces every page it
replayed, then logs a checkpoint with an empty dirty page table and
points the file header at it. The open takes it once it has read the
marks and the damage list.

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

An `Order`, `Orphan` or `Missing` finding of `verify` quarantines the
index it names. A page no heap or index holds, such as one of the
catalog or the allocator, quarantines the whole database as a commit log
page does.

A commit log page that fails verification quarantines the whole
database, since the visibility of every relation reads the commit log,
and so does a retained log the open cannot read whole when it settles
the ids a crash left in progress:
every transaction is refused with the corruption fault, and the
database opens for `verify` and `dump` only
([mvcc.md §4.2](mvcc.md#42-the-commit-log)).

Opening a file under that mark, postconditions:

- Every call fails with `Quarantined` before its text is checked, except
  one `verify` alone. As a call it always runs. As a task it needs a session,
  and `session` reads the roles: on a file whose catalog or commit log cannot
  be read, only a session issued before the damage has one.
- Nothing past the mark is written. A damage found from then on is recorded
  in memory only. A checkpoint still folds into the damage list every
  record the log holds, including those replay reads at the open of a file
  already marked. The open resolves no id, frees no page,
  allocates no counters page, applies no migration and compares no
  migration of the given log with the file's. No transaction with an id
  begins, so `verify` keeps no findings and a refused call records no `Denied`
  audit row. Once the mark is durable, this holds of every later open. An open
  that finds the damage only after opening an intact commit log has already
  resolved the ids left in progress.
- Pages 0 and 1 and the log may change on open and close: recovery and the
  close's checkpoint write them. Every other page but the page of marks and
  the damage list stays as it was.
- `verify` answers at least one `Structure` row naming the mark, so a mark
  over a page replay has healed never reads as sound. A mark on one heap or
  index adds no row.
- The open trusts a row of the catalog only when the status of its writer and
  deleter is read from an undamaged status page, or a commit record of the
  retained log proves it committed. A row of an id neither settles leaves the
  whole catalog unread: an uncommitted migration never reads as applied.
- When a page's damage stops the catalog being read, the file still opens,
  with only the engine's own declarations and a commit log of no pages.
  `verify` then checks every page and the retained log. It reports `Checksum`,
  `StalePage` and `Structure` by page number and a `Structure` finding that
  says only those were checked. It cannot report `Order`, `Orphan`, `Missing`
  or `Violation`, and refuses `verify r`.
- A failed open that found no damaged page, or an unreadable one, still fails.
  So do a file header with neither copy valid, a file that is no adtb file, a
  retained log recovery refuses, and a row that fails to decode on a page
  that verifies.

Quarantine is a mark the storage layer sets on one heap, one index or
the whole database, and keeps until the object is repaired:

```text
pager.quarantine(Quarantine::Heap(id) | Index(id) | Database) -> Result<(), PageError>
                                                  // caller holds no page latch
pager.release(object) -> Result<(), PageError>    // once repaired
pager.replace_contents(page, from, Option<object>) -> Result<(), PageError>
                                                  // page takes from's contents, from is freed
                                                  // and any object's mark goes, in one batch
pager.scrap(page) -> Result<(), PageError>        // free a damaged page, overwriting it
pager.is_quarantined(object) -> bool              // its own mark or the database's
pager.quarantined() -> BTreeSet<Quarantine>
```

The marks live on one page of the file, of kind quarantine marks,
whose id the file header records (§1.2); the database's creation
allocates it, and the damage list (§6) beside it. `verify` and a scrub
report each page the damage list or a record since names as a finding of
that page with its damage, and each stale log block it holds as a log
finding; a full list is a finding of the whole file. A settled status
page is reported and marks nothing. A mark or release is a logged change of that page,
durable when the call returns, so the marks survive a crash and a
restart and are read back at open. A heap or index stays quarantined
across restarts until `release`. A marks page that fails verification
at open quarantines the database, and `quarantine` and `release` then
fail with its damage.

`quarantine` of an object already marked writes nothing, and every caller
returns with the mark durable. When it fails, the mark still holds in
memory until the pager closes. An I/O failure reading the marks' page at
open fails the open and marks nothing.

The marks of damaged pages are derived from the damage records (§6). One
mapper takes each record not yet mapped this open, maps its page to the
object holding it and marks that object; a record counts as mapped only once
its marks are set. Which records are mapped is memory state, empty at every
open, so every open maps every record again and a mark a crash lost comes
back. The mapper runs at three points and nowhere else:

- every open, once the catalog loads, whether the open succeeds or fails;
- the end of an API call, holding no page latch and not the serialization
  lock, whatever the call returns;
- a commit, under its hold of the serialization lock and before it checks
  the marks, while any record is unmapped. It maps only the records
  unmapped when its mapping begins, and leaves out a record whose last
  mapping was incomplete. Each record it maps runs the walks of every heap,
  large-value store and index, each walk up to its bounded number of runs.
  Damage a read of those walks detects, on a page with no record when the
  mapping began, maps to the database with no walk of its own. A record
  another call appends meanwhile, even on a page those walks then read,
  waits for that call's end, the next commit or the next open.

A settled status page, a stale log block and `Damage::Unreadable` map to
nothing. A full damage list maps every record to the database. An `Order`,
`Orphan` or `Missing` finding, and an index entry naming an unused slot,
mark their index at the call that finds them. The failure a call returns
names what its page's record was mapped to, and a failure of the
transactions layer names the database once the database is marked. A page maps as follows:

- a page of a constructor's heap, current or history, marks that heap; a
  page of an index marks that index, and a page of a history key index
  marks its key index;
- a mark comes only from a page that fails its checksum or log position.
  A walk takes no latch, so a shape it cannot follow proves nothing, the
  object may be changing: the walk runs again, a bounded number of times;
- a walk that stops on damage on two runs in a row marks that object;
- a walk that stops on an I/O failure, or still stops on a shape when its
  runs are spent, leaves the mapping incomplete. An incomplete mapping that
  found no holder marks nothing, not even the database, and the call fails
  as a storage failure naming the page; its record stays unmapped, and the
  next open or end of a call maps it again. A commit does not, and the
  page still refuses every read, so no commit writes through it;
- a page of a relation of an engine module, a page of the large-value
  store every heap shares, and an allocated page a complete walk of every
  heap and index does not reach mark the database;
- a page the allocation bitmap calls free, read before and after the walk,
  marks nothing, and a page of the store of kept rows marks nothing. A
  page whose bitmap page is refused counts as allocated. A walk
  of that store stopped by damage leaves the mapping incomplete;
- `Damage::Unreadable`, an I/O failure, marks nothing and stays a storage
  failure.

An open maps with the catalog's structures, and marks nothing for a page
they do not reach. When the catalog itself does not open, only a page of its
root, its kept-rows heap or the commit log is mapped, to the database.

An `Order`, `Orphan` or `Missing` finding comes from a check that holds off
vacuum across its index, and vacuum is the one writer that frees a page of a
tree, so concurrent writers cannot make the check's walks follow a freed page.

The planner and the
transactions layer read the marks and rely on them as follows
([execution.md §6](execution.md#6-quarantine)):

- a call whose read set
  ([../spec/queries.md §12.4](../spec/queries.md#124-the-constructors-a-statement-reads))
  includes a quarantined heap fails, whether or not its plan prunes
  that heap;
- no plan uses a quarantined index;
- a write is refused where a constraint or reference check must read
  the quarantined heap, or where it changes a constructor relation a
  quarantined index covers;
- a commit is refused by that rule, applied to the constructors its
  transaction's statements that succeeded wrote. It aborts the transaction
  and leaves nothing visible. What the transaction only read plays no part,
  and a transaction that wrote nothing commits.

Commits and marks set by the write path are ordered. A commit checks the
marks and writes its commit record under one hold of the serialization lock,
and the write path sets a mark under that lock. Postconditions:

- a mark set before that hold refuses the commit;
- a commit whose hold came first stands, ordered before the mark, with its
  commit record below the LSN of the mark's change to the marks' page. A
  repair that starts after the mark reads what it committed.

The error a call receives is in
[../spec/catalog.md §4](../spec/catalog.md#4-error-sums).

```adt
-- identities.Apple quarantined
from i in identities.(EmailPass | Google) select i.id    -- runs
from i in identities select i.id                         -- fails
```

A quarantined object is restored by reindexing (an index) or by
restoring a backup to a new file and swapping it in (a heap). The swap
of a rebuilt index removes the index's mark in the batch that changes its
root page, so a crash leaves the old index with its mark or the new one
without. Neither call reads a damaged page it overwrites; it logs a
full-page image with an unknown previous LSN, which starts the page's
chain of records anew (§7).

Rulings: ops/halt-scope/heap, spec/reads/static/static-types, spec/quarantine/index/cost-only, spec/quarantine/verify/yes, spec/quarantine/unowned/database

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
page memory under one lock. The pager holds the expected LSN of every
page (§3) from the open: the page LSN map's entries in an array of 8
bytes per page of the file, about 1 GiB per TiB of file at 8 KiB pages,
and the LSNs past them that the retained log or a later record gives in
a hash table of one entry per page those records name. Recovery builds
both as its last step, before the open reads any page recovery did not
replay: it reads the whole map, then scans the retained log to the end
recovery kept, decoding only the page and LSN of each record. A segment
it cannot list, open or read, a block missing or a batch whose records
do not decode before that end quarantines the whole database for that
open and writes nothing, no mark included; a map page the open cannot
read does the same. While the database is held quarantined this way,
no page read in is checked against an expected LSN. Damaging a page
outside recovery writes its memory first when it holds changes the file
lacks, then drops it, so its entry leaves the dirty page table at the
next force. On a file quarantined whole the memory is dropped unwritten
and the entry stays, so a checkpoint's redo point stays at or before its
oldest change the file lacks and a recycle removes no segment from it
on. A recycle raises the expected LSNs of the pages it folded to their
map entries, dropping a later LSN that is no longer later. The page LSN map
(§6) is read through the pool like any page, from the root to a leaf,
each page checked against its parent. Group commit is §10.1.
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
