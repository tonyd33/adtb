# Viewstamped Replication Revisited (Liskov, Cowling)

kind: paper
home: https://pmg.csail.mit.edu/papers/vr-revisited.pdf

## Protocol

- MIT-CSAIL-TR-2012-021. Primary-backup replication with view changes; primary of a view chosen deterministically from the view number (round robin). [U]
- Replicas need not write to disk before replying; durability comes from failure independence of a quorum, with a recovery protocol for a restarted replica. [U]
- Includes reconfiguration (epochs). [U]
- PDF unreachable 2026-10-25 (connection refused, 429 from dspace). [V 2026-10-25]
