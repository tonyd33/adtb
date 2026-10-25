---
title: Where isolation histories are produced
pick: some
raised-by: [project/testing/isolation-check]
options:
  in-sim:
    title: Simulated clients inside the deterministic simulator
    needs: [buffer/test/dst-threads/sim-scheduler]
    links:
      - {rel: prior-art, to: prior/buffer-sim}
  real-process:
    title: Real server processes killed, paused and restarted, Jepsen style
    links:
      - {rel: prior-art, to: prior/project-elle}
---
The simulator replays a failing seed; a real process finds what the
simulation does not model.

## project/testing/where/in-sim

Each simulated client runs registered transactions; the history is
collected from the simulator and checked as Elle checks Jepsen's. A
failing cycle comes with the seed that produced it.

## project/testing/where/real-process

Jepsen drives real nodes with process kills, pauses and clock skew;
here, the server of `engine/embedded-server` under `kill -9` and
restart, through the binary wire protocol.
