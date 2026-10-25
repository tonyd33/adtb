# Calvin (Thomson et al.)

kind: paper
home: https://dl.acm.org/doi/10.1145/2213836.2213838

## Protocol

- SIGMOD 2012. A sequencer fixes a global transaction order before execution; locks taken in that order make execution deterministic. [U]
- Read and write sets must be known in advance; dependent transactions run a reconnaissance pass (OLLP). [U]
- Replicates transaction inputs, not effects; determinism lets replicas stay consistent by replaying input. [V 2026-10-25] http://cs.yale.edu/homes/thomson/publications/calvin-sigmod12.pdf
- Sequencers collect requests in 10 ms epochs into batches; batch replication is asynchronous (master replica forwards batches) or Paxos-based synchronous. [V 2026-10-25] same, sect. 3.1
- Dependent transactions (sets not derivable statically) use OLLP: an unreplicated low-isolation reconnaissance read predicts the read/write set; the real transaction restarts deterministically if the prediction was wrong. [V 2026-10-25] same, sect. 3.2.1
- Determinism plus checkpointing removes physical REDO logging. [V 2026-10-25] same, sect. 1
