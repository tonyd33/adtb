# Indexes

The index layer provides constructor-set indexes over the heaps of
[mvcc.md](mvcc.md). Its consumers are the transactions layer
(uniqueness checks, SIREAD range locks, vacuum), the planner and
executor of [execution.md](execution.md), and the maintenance of
[maintenance.md](maintenance.md).

## 1. Constructor-set indexes

Every index, unique constraint, key and reference target is one
mechanism: an index declared over a constructor set `S` of one
relation variable, holding an entry `(key, ctor, tid)` for every
indexed version of a row whose constructor is in `S`. The key index
ranges over every constructor; `unique (gid) in Google` over
`{Google}`; a reference typed `Key identities.OAuth` targets an index
over `{Google, Apple}`.

Invariant: a sum's key is always covered by a constructor-set index
over every constructor of the sum, its key index.

Rulings: index/ctor-set

## 2. Entries

Entries sort by key, then constructor id, then TID. Consumers may rely
on:

- All entries of one key are adjacent, so a uniqueness probe reads one
  range.
- Within a key, one constructor's entries are adjacent, so a probe for
  `Key identities.Google` seeks `(k, Google)`.
- Every entry is unique, because `(ctor, tid)` trails the key.

```text
key index of identities:
  (7, EmailPass, (12,3))  (7, Google, (4,1))  (8, Google, (4,2))
```

The constructor id is encoded as an order-preserving, prefix-free
varint, `ovarint`, not LEB128: an id below 240 is one byte, the id; a
larger id is the byte `239 + n` followed by the id as `n` big-endian
bytes, `n` from 1 to 4 the fewest that hold it. The whole entry is one
byte string compared with `memcmp`:

```text
entry bytes = encode(key) ++ ovarint(ctor) ++ be48(page) ++ be16(slot)
```

The TID format is that of [mvcc.md §2](mvcc.md#2-tids).

Rulings: btree/entry-order/after-key, btree/ctor-width/uvarint

## 3. Key encoding

Index keys are byte strings with three properties a consumer may rely
on:

- their `memcmp` order is the key order;
- values equal in the language encode to identical bytes;
- the encoding decodes back to the value.

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

A collated order is an expression index over a collation function's
sort key
([../spec/derived.md §5](../spec/derived.md#5-computed-fields)); `Text`
keys themselves compare by code point only, so every key decodes.

Rulings: btree/keyenc/memcmp, btree/keyenc/canon/canonicalize, btree/varlen/escape-term, btree/keyenc/lossy/code-point, btree/sum-key/tag-payload

## 4. Order keys

A sum's tag is encoded by an order key: a short byte string per
constructor kept in the catalog.

Invariants:

- Order keys follow the sum's declared order, so an index scan of an
  ordered sum field is in the language's order. Optionality is a
  constructor, so where `None` sorts is its place in the declared
  order.
- A constructor inserted between two others gets a string between
  theirs, so no stored key changes.
- Reordering rewrites only the keys holding the moved constructor.

```adt
type Status ordered = Draft | Sent { at: Instant } | Paid { at: Instant }
```

has order keys Draft 40, Sent 80 and Paid C0, and `Sent { at: t }`
encodes as

```text
80 00 | be64(t xor 2^63)
```

Rulings: btree/sum-tag/order-key

## 5. Hashing

The hash of a value is a hash of its canonical key encoding (§3), in
which equal values have equal bytes. Hashing is therefore consistent
with `==` for every type, sums and records included. A hash that is
persisted (partitioning, hash indexes) uses a fixed function, versioned
with the file format; a hash used only in memory may be seeded per
process.

Rulings: types/hash/key-bytes

## 6. Index kinds

### 6.1 Indexes on value-level sum fields

A value-level sum field may be indexed three ways:

| Declaration | Key | Use |
| --- | --- | --- |
| `index (status)` | the whole value, as §3 | equality, constructor prefixes, ranges on ordered sums |
| `index (tag(status))` | the order key only | constructor tests |
| `unique (status.Paid.ref)` | one constructor's field; entries only for rows holding that constructor | used by an arm whose pattern names the constructor |

The planner may use an index on a constructor field only where a
query's arm pattern names that constructor; the implication is
syntactic and needs no predicate prover.

Expression indexes and the derived indexes of full-text and spatial
search are B-link trees over relations derived by a function
([../spec/derived.md §5](../spec/derived.md#5-computed-fields),
[../spec/interop.md](../spec/interop.md)) and inherit everything in
this file.

Rulings: btree/sum-field/whole, btree/sum-field/by-tag, btree/sum-field/ctor-field, btree/sum-field/implied/arm-scoped

### 6.2 Hash indexes

A constructor-set index whose every use is an equality probe may be a
hash index. A bucket entry is `(hash u32, ctor, tid)`; equal hashes are
rechecked against the version in the heap the entry names. A hash
index can enforce uniqueness, has no ordered scan and serves no sum
scan merge.

TODO: whether a hash index is chosen by the engine or declared by the schema is undecided.

Rulings: btree/hash/eq-unique

## 7. Building and rebuilding

An index over several heaps is built by scanning and sorting each heap
into a run of entries independently and in parallel, then merging the
runs into leaves filled left to right, building each level from the
first keys of the level below. A heap already sorted by the key skips
its sort. A unique index reports adjacent equal keys in the merge as
violations.

A split or merge of constructors moves rows to the new constructors'
heaps and, in every index of the sum, deletes the moved rows' old
entries and inserts entries naming the new constructor ids, batched by
key order.

A load chooses, per load, how to maintain each index: a bulk build from
sorted runs into an index over an empty constructor set, a sorted merge
of the load's entries in key order into a populated index, or retail
inserts ([maintenance.md §6](maintenance.md#6-checking-a-large-change-set)).

An index is rebuilt either online, by building a new index from sorted
runs of each heap while writes continue, catching up with writes made
meanwhile, then swapping, staged as a schema job
([schema-change.md §2](schema-change.md#2-staged-states)); or offline,
with writes to every constructor of the index's set blocked. One
constructor's entries are spread through the tree, so an index is
always rebuilt whole.

Removing a constructor leaves its entries in every index until vacuum
clears them ([mvcc.md §10.4](mvcc.md#104-removing-a-constructor)).

Rulings: btree/build/merge-runs, btree/ctor-rewrite/rewrite, interop/load/index-path/bulk-build, interop/load/index-path/sorted-merge, interop/load/index-path/retail, ops/reindex/online, ops/reindex/offline, union-index/drop-ctor/after-vacuum

## 8. Notes for the implementation

Non-binding.

### 8.1 B-link trees

Ordered indexes are B+-trees with right-links and high keys (B-link
trees). Every page but the rightmost of its level carries a high key;
a descent whose key exceeds a page's high key moves right, so a split
holds latches on one level at a time. Sequential keys append at the
right edge.

Latching is optimistic lock coupling on the page's state word
([sys.md §3](sys.md#3-page-state-word)): a reader records the version,
reads, and restarts from the root if the version changed or the page
was locked; a writer takes the word exclusive and increments the
version on release. A descent never writes a state word; a scan that
sets kill bits takes the leaf's word exclusive for the write, as a
writer does. Optimistic reads load page bytes through atomic loads
into local copies and bounds-check every length and offset read before
use; no reference to a page another thread may write is formed.

A leaf stores equal keys once followed by a sorted array of
`(ctor, tid)` pairs. Deduplication runs lazily, when a leaf would
otherwise split.

```text
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
it, and so logs a full-page image first
([storage.md §5.2](storage.md#52-full-page-images)).

Underfull pages are never merged. An empty page is unlinked from its
parent, then from its siblings, and recycled once no snapshot older
than its deletion remains. The rightmost page of a level is never
deleted.

TODO: split point choice and key compression are deferred.

Rulings: btree/structure/blink, btree/latch/olc, btree/latch/racy/atomic-reads, btree/posting/posting-pairs, btree/cleanup/kill-bit, btree/cleanup/bottom-up, btree/merge/empty-only
