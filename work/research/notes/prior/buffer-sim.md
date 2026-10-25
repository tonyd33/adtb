---
title: Deterministic simulation testing (FoundationDB, TigerBeetle VOPR)
---
FoundationDB runs a whole cluster in one single-threaded process on its
Flow runtime, which has a production and a simulated executor; all
time, network and disk go through it, so a seed reproduces a run.
TigerBeetle's VOPR does the same with storage faults (corruption,
misdirection, latent errors, torn writes) and accelerated time.
Rebello et al.'s CuttleFS injects file-system-specific fsync failure
behaviour under real applications.
https://apple.github.io/foundationdb/testing.html
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/concepts/safety.md
https://www.usenix.org/system/files/atc20-rebello.pdf
