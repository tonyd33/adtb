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

Invariant: every reference field of a relation variable of a user
module is the leading field of an index over every constructor holding
the field: a declared key, unique or index, or the field's implied
index ([../spec/constraints.md §2.1](../spec/constraints.md#21-keys-uniques-and-references)).
The reference fields and the index serving each are those
`Schema::reference_fields` returns ([kernel.md §6](kernel.md#6-the-schema-a-term-refers-to)).
The write path finds the referrers of a deleted or moved target by
probing it.

Invariant: an index's root page is fixed when the index is created and
never changes while the index exists, so the catalog names an index by
its root page id.

Rulings: index/ctor-set, constraint/admit/reference-index/implied

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
bytes, `n` from 1 to 4 the fewest that hold it. A decoder refuses every
other form. The whole entry is one byte string compared with `memcmp`:

```text
entry bytes = encode(key) ++ ovarint(ctor) ++ be48(page) ++ be16(slot)
```

The TID format is that of [mvcc.md §2](mvcc.md#2-tids). The key's
encoding is prefix-free, so `encode(k) ++ ovarint(c)` is a prefix of
exactly the entries of key `k` and constructor `c`.

`Index::check_key(page_size, key, ctor)` checks whether an entry fits
before a TID is allocated, without reading or changing pages. It returns
`TooLong { len, max }` with the encoded entry length and maximum in bytes.
The entry length includes the encoded constructor id and eight-byte TID.
An entry whose length equals the maximum fits.

Preconditions:

- `ctor` belongs to the index's constructor set.
- `key` has the index's key schema.
- `page_size` is the database's page size.

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
| `F32`, `F64` | The canonical value (no -0.0; one NaN, the quiet NaN `7FC00000` or `7FF8000000000000` with the sign clear) as big-endian IEEE bits; if the sign bit is set every bit is inverted, otherwise only the sign bit; NaN sorts above every other value |
| `Decimal s` | The coefficient `n` of `n / 10^s` as a 128-bit integer, encoded as `I8` to `I64` are; the scale is in the type and is not encoded |
| `Instant`, `Duration` | Microseconds as an `I64` |
| `Date` | Days since 1970-01-01 as an `I32` |
| `Time` | Microseconds since midnight as an `I64` |
| `DateTime` | Microseconds since 1970-01-01T00:00:00 civil as an `I64` |
| `ZonedDateTime` | The `DateTime` encoding, then the zone name as `Text`; the type has no order, so the encoding serves equality and uniqueness only |
| `Period` | The month count, then the day count, each as an `I32`; equality and uniqueness only, as for `ZonedDateTime` |
| `Text` | NFC UTF-8 bytes, each `00` written `00 FF`, terminated by `00 01`; the order is code point order |
| `Bytes`, `SortKey` | As `Text`, without normalization |
| `Key r` | The encoding of the target's key |
| newtype | The encoding of its representation |
| record | Its fields' encodings concatenated in field order |
| sum value | The constructor's order key, then `00`, then the header fields and the constructor's fields in order, as a record |
| anonymous record | Its fields' encodings concatenated in name order |
| anonymous sum value | The constructor's index among the sum's constructors in name order as an `ovarint`, then its fields in name order, as a record |

Every encoding is prefix-free: no value's encoding is a proper prefix
of another's of the same type. A `00` inside a variable-length value is
always followed by `FF` and its terminator `00` by `01`, so a decoder
knows where the value ends whatever follows. A terminator of `00` alone
is not prefix-free in a composite key: `("a", 32767 : I16)` would encode
as `61 00 FF FF` and sort above `("a\0", -32768 : I16)`, `61 00 FF 00 00 ...`.

A descending component of a key, as in an order key list with `desc`,
is the ascending encoding with every byte inverted. Inverting a
prefix-free encoding reverses its order and keeps it prefix-free.

A decoder refuses every byte string the encoding does not produce: a
truncated value, an escape other than `00 FF` or `00 01`, text that is
not UTF-8, a float that is not canonical, a decimal of more than 38
digits, a `Time` outside a day, a `Date` whose midnight is not an
`Instant`, an order key the sum does not have, a constructor index an
anonymous sum does not have, and trailing bytes. A
decoder does not check that text is in NFC.

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
- An order key is a nonempty byte string that holds no `00` byte and
  does not end in `01`. Between any two such strings, and below and
  above any one, lies another, so insertion never runs out of room. A
  key without `00` needs no escaping, and its terminator `00` sorts it
  below every longer key it prefixes.

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

The persisted function is XXH64 with seed 0 over the key bytes. A
32-bit hash is the high 32 bits of that value.

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

An index's key is a list of elements, each a field, a path into a
value-level sum field, or an expression of the row. Consumers may rely
on:

- A row has an entry only when every value on each of its paths holds
  the constructor the path names; a unique constrains no other row.
- An expression element's component is the expression's value on the
  row under the current code, encoded as its type is in §3.
- An expression that faults on a row fails the write of that row with
  the fault, and refuses a migration that would build an entry for it.
- An entry may name a version whose key no longer has the entry's
  value, so a reader of a probe evaluates each element of the key on
  the version it fetches.

Rulings: btree/sum-field/whole, btree/sum-field/by-tag, btree/sum-field/ctor-field, btree/sum-field/implied/arm-scoped

### 6.2 Hash indexes

A constructor-set index whose every use is an equality probe may be a
hash index. A bucket entry is `(hash u32, ctor, tid)`; equal hashes are
rechecked against the version in the heap the entry names. A hash
index can enforce uniqueness, has no ordered scan and serves no sum
scan merge.

The schema declares no hash index; the engine chooses one from the
uses of a constructor-set index and may change its choice by a rebuild
(§7).

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
meanwhile, then swapping; or offline, with writes to the index's relation
held until the rebuild ends. One constructor's entries are spread through
the tree, so an index is always rebuilt whole. A rebuild reads only
heaps, so a damaged index can be rebuilt.

Postconditions of a rebuild, for subsystems reading or writing the index:

- The root page keeps its id: the swap gives it the contents of the new
  tree's top page. A SIREAD lock on the index, a key range on that id,
  holds across the swap.
- A writer writes a version before its entry and holds the index's latch
  across the entry. The last catch-up holds that latch from before it
  reads the heaps until the swap, so no window loses an entry.
- The new tree may hold an entry a writer is about to insert, and an
  insert of a held entry changes nothing. A uniqueness probe passes over
  the writer's own entry.
- Each old page is marked deleted after the pages leading to it, then
  freed: a descent still in the old tree starts again from the root.
- Vacuum does not run during a rebuild. One rebuild runs at a time.
- A failed or stopped rebuild frees what it allocated and keeps the old
  index. The catalog root's `changing` marks the rebuild from its first
  allocation, so an open after a crash frees what no structure names.
- A rebuild of a key index judges its history key index at the swap, under
  the latch, and rebuilds that tree from the history heaps if it is damaged,
  in a swap of its own that leaves the mark. The mark goes only with the swap
  of the key index, once both trees are sound. A failed history build keeps
  the mark.
- A writer gated by an offline rebuild holds no latch and no worker: a call
  that suspends suspends on the reserved transaction `u64::MAX`, ended when
  the gate opens, and any other looks at the gate, its stops and the close
  every few milliseconds. A close refuses it with `Cancelled`.

Removing a constructor leaves its entries in every index until vacuum
clears them ([mvcc.md §10.4](mvcc.md#104-removing-a-constructor)).

Rulings: btree/build/merge-runs, btree/ctor-rewrite/rewrite, interop/load/index-path/bulk-build, interop/load/index-path/sorted-merge, interop/load/index-path/retail, ops/reindex/online, ops/reindex/offline, union-index/drop-ctor/after-vacuum

## 8. Notes for the implementation

Non-binding.

### 8.1 B-link trees

Ordered indexes are B+-trees with right-links and fence keys (B-link
trees). Every page but the rightmost of its level carries a high key,
and every page but the leftmost of its level a low key, its left
sibling's high key. A descent whose key reaches a page's high key moves
right, so a split never holds a latch while it waits for one on the
level above. When a page that is the rightmost of its level splits for
an item that goes after all of its others, its left half keeps all but
its last item, so sequential keys fill the pages they leave behind.

A leaf split's separator, which becomes the left half's high key, the
right half's low key and the downlink's key in the parent, is the
shortest prefix of the right half's first entry that is above the left
half's last. An inner split's separator is the first key of its right
half, unshortened, since it is the low key of the right half's first
child.

Latching is optimistic lock coupling over the pager's latches
([storage.md §2.4](storage.md#24-latches)). A descent reads each page
optimistically and uses what it read only once the read validates; a
read that fails validation is retried on the same page, and after a few
failures takes the page's shared latch instead. A descent holds no latch
or version across levels: a page that split after its parent was read
is reached through its left sibling's right link. A descent never
writes a page; a scan that sets kill bits takes the leaf's exclusive
latch for the write, as a writer does. Optimistic reads load page bytes
through atomic loads into local copies and bounds-check every length
and offset read before use; no reference to a page another thread may
write is formed.

A page id read anywhere may be stale by the time it is followed: the
page may have been unlinked and freed, and allocated again to any
owner. A reader trusts a page it reaches only once a validated read
shows it a live page of the tree, at the level it expects, whose low
key is at or below the key sought; a page that fails while the page that
led to it is unchanged is malformed, and otherwise the reader starts
again from the root. A writer checks the same under the page's
exclusive latch. Pages of the leaf level are the only ones unlinked.

Invariants:

- The low and high keys of the live pages of a level partition the key
  space; each page's low key is its left sibling's high key.
- Latches are taken top-down and left to right. A thread holding a
  latch on a page takes latches only on its right siblings, on pages
  of lower levels, and on pages it has just allocated.
- A tree operation holds at most four latches at once, besides the
  allocation bitmap page an allocating or freeing update latches for
  itself.

Postconditions:

- An insert or delete descends optimistically, then takes the
  exclusive latch on the leaf and moves right, releasing each latch
  before taking the next, until the leaf's range holds the key; a page
  found unlinked, or whose low key is above the key, sends it back to
  the root. It decides whether the entry is present under that latch,
  so each insert, delete and lookup takes effect at one instant between
  its call and its return.
- A scan holds a key between leaves, the high key of the last leaf it
  read, and never a page id.
- A pass that deletes entries by a predicate walks the leaves left to
  right, judging and changing each under its exclusive latch and
  releasing it before latching the next. Every entry present when the
  pass starts and still present when it ends has been judged.
- A split changes the page, its new right sibling and the page whose
  downlink it carries in one change under their exclusive latches,
  releases them, and then latches the parent level from the page its
  descent passed there, or from the root.
- A change that adds a downlink latches the parent, then the child,
  and checks the child's flag under its latch: a clear flag means
  another thread added the downlink, and nothing is changed. So a
  split that a crash or another thread left incomplete is finished
  once, under the same latches as any split.

A leaf stores equal keys once followed by a sorted array of
`(ctor, tid)` pairs, a posting. Deduplication runs lazily, when a leaf
would otherwise split: the leaf's runs of entries sharing a key are
rewritten as postings, and the leaf splits only if that leaves too
little room. Each remainder carries a one-byte length, and a posting
grows to at most a fifth of a page, so a posting splits as any item
does; an entry that lands inside a full posting cuts it in two around
itself.

```text
posting:  key | n u16 | (len u8, ovarint ctor, tid 8 B) * n
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

TODO: kill bits wait on the unlogged change of storage.md §2.3; until
then an inserter judges a full leaf's entries through the caller.

Preconditions:

- A caller that judges entries dead before a split holds no latch on a
  heap page across the index call; it reads heap pages under shared
  latches while the leaf's exclusive latch is held.

A root that splits moves its two halves to new pages and becomes their
parent, so the root page never moves. Any other split is logged one
level at a time: the page and its new right sibling in one change, then
the separator in the parent. A crash between the two leaves the new
page without a downlink; descents still reach it through its left
sibling's right link.

Invariants:

- A page carries the incomplete-split flag exactly when its downlink is
  not in its parent. The split that creates a page below the root sets
  the flag in the same change; the change that adds the page's downlink
  to its parent clears it.
- The pages a root split creates carry downlinks from the change that
  creates them, and never carry the flag.

Postconditions:

- An insert whose descent reaches a flagged page by a right link first
  adds that page's downlink, the left sibling's high key as separator,
  to the parent level, then descends again.
- A repair pass walks each level below the root by its right links and
  finishes each flagged page it meets. After recovery and one repair
  pass, every page but the root has its downlink.
- `verify` refuses a page that lacks a downlink without carrying the
  flag, or carries the flag and has one. It reads each page on its own,
  so it judges the tree only while no writer runs; a repair pass may
  run beside writers.

Vacuum's index pass walks the leaves left to right, finding each by
the high key of the one before, and deletes, in one update per leaf,
every entry whose `(ctor, tid)` is in its dead set; a leaf it empties
is unlinked as below.

Underfull pages are never merged. A leaf that a delete or a predicate
pass empties is unlinked and freed in one change, the freeing logged in
the same batch as the unlink: its downlink leaves the parent, and its
range goes to its left sibling under the same parent, whose high key
becomes the leaf's, or, when there is none or it has no room for that
key, to its right sibling, whose low key and separator become the
leaf's. A parent's first child goes to its right sibling; its left
sibling, under the parent before, only takes its right link, and is
found by a descent to the keys just below the leaf's low key. The
change latches the parent, then the left sibling, then the leaf and its
right sibling. A leaf is left in place when it is the rightmost of its
level, its parent's only child, lacks its downlink, or neither sibling
can take its range; inner pages are never unlinked, so height never
shrinks.

The page is freed at once rather than after every reader that may hold
its id has finished, because no reader trusts an id it holds: the fence
check above refuses a freed page, or one allocated again, and sends the
reader back to the root. An epoch or snapshot horizon would add a
registry that every descent writes to, which the fence keys make
unnecessary.

TODO: split point choice and prefix compression are deferred.

Rulings: btree/structure/blink, btree/latch/olc, btree/latch/racy/atomic-reads, btree/posting/posting-pairs, btree/cleanup/kill-bit, btree/cleanup/bottom-up, btree/merge/empty-only
