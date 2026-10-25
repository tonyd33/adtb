---
title: Calvin deterministic replication
---
Thomson et al. SIGMOD 2012: replicate transaction inputs, not effects.
Sequencers batch requests in 10 ms epochs, replicated async or by
Paxos; deterministic locking in sequence order makes replicas agree.
Dependent transactions predict their sets with a reconnaissance read
(OLLP) and restart if wrong.
http://cs.yale.edu/homes/thomson/publications/calvin-sigmod12.pdf
