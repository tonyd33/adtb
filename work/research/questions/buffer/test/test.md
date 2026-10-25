---
title: How durability is tested
pick: some
raised-by: [substrate/new-engine]
options:
  dst:
    title: Deterministic simulation of the whole engine with injected storage faults
    links:
      - {rel: prior-art, to: prior/buffer-sim}
  fault-fs:
    title: A fault-injecting file system under the real engine
    links:
      - {rel: prior-art, to: prior/buffer-fsync}
  crash-replay:
    title: Record the block-level write stream and recover from every crash prefix
  power-cut:
    title: Physical power loss on real devices
---
The fault model (`buffer/faults`) names what may happen; testing shows
recovery survives it. Rebello et al.: application recovery is rarely
tested against low-level block faults.

## buffer/test/dst

FoundationDB Flow and TigerBeetle VOPR: time, I/O, network and
randomness behind one interface; a seed replays a run exactly.
Storage double: an in-memory disk that tears, corrupts, misdirects,
drops writes after the last sync, and delays completions.

```
trait Storage {
  fn submit(&mut self, op: Io) -> Ticket;
  fn poll(&mut self) -> Option<(Ticket, Result<usize, IoError>)>;
}
sim: SimStorage { seed, fault: { torn_p, corrupt_p, misdirect_p, lose_unsynced: true } }
```
https://apple.github.io/foundationdb/testing.html

## buffer/test/fault-fs

CuttleFS (Rebello et al.): FUSE with its own page cache emulating
ext4, XFS or Btrfs fsync-failure behaviour. Tests the real binary, not
deterministic.

## buffer/test/crash-replay

Log every block write and flush between engine and device; for each
flush boundary and each subset of unflushed writes, build the disk
image and run recovery (ALICE, CrashMonkey shape, unverified).

## buffer/test/power-cut

Catches device lies about flushes; slow, few runs.
