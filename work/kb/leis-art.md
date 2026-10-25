# Leis et al., Adaptive Radix Tree and its synchronization

kind: paper
home: https://db.in.tum.de/~leis/papers/ART.pdf

## ART (ICDE 2013)

- Inner node types: Node4 (2-4 children, 16+4+4*8 = 52 bytes), Node16 (5-16, 16+16+16*8 = 160), Node48 (17-48, 256-byte child index plus 48 pointers, 656), Node256 (49-256, 16+256*8 = 2064); 16-byte header holds type, child count and compressed path. Node16 searched with SIMD. [V 2026-10-25] https://db.in.tum.de/~leis/papers/ART.pdf
- Lazy expansion (inner nodes created only to distinguish two leaves) and path compression (one-child nodes removed, prefix stored in node) bound worst-case space at 52 bytes per key for arbitrarily long keys; 34 with six node types; best case 8.1 bytes for dense integers. [V 2026-10-25] same
- Binary-comparable keys: t with x<y iff memcmp(t(x),t(y))<0; unsigned ints big-endian; signed flip sign bit; floats ranked by 10 classes; strings terminated by a byte not occurring elsewhere since keys must not be prefixes of other keys; null by widening some values; compound keys by concatenating transformed attributes. [V 2026-10-25] same
- Leaves assume unique keys; non-unique indexes append the tuple identifier as a tie-breaker. [V 2026-10-25] same

## The ART of Practical Synchronization (DaMoN 2016)

- Optimistic Lock Coupling: each node holds a lock, version counter and obsolete flag in one 64-bit word; writers lock and increment the version; readers take no lock, read the version, and validate it after reading (readUnlockOrRestart), restarting from the root on change; validation needed before dereferencing an optimistically read pointer. Deleted nodes marked obsolete, freed through epoch-based reclamation. Restarts can be bounded by falling back to pessimistic locking. [V 2026-10-25] https://db.in.tum.de/~leis/papers/artsync.pdf
- ROWEX: writers take locks, readers never lock or check versions and never restart; needs nodes modified with atomic stores so a reader always sees a consistent node. [V 2026-10-25] same
- OLC is stated to apply to B-trees as well. [V 2026-10-25] same
