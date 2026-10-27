---
created-at: 2026-10-26
status: open
kind: feature
priority: 80
summary: "parallel agents' full test runs contend for 16 cores, so a 40 second suite takes 4 to 10 minutes and times tests out"
tags:
  - tooling
  - devex
---

# Feature: one full test run at a time across workspaces

## Report

Agents run the full suite in parallel jj workspaces on one 16-core machine. Load during the 2026-10-26 tracks had a median of 22 and peaked at 70. The default run takes 32 to 86 seconds alone (test-suite-time), but each track's final run took 236 to 600 seconds: sound 236s, qual4 306s, eadhoc 504s, evo 539s, strength 426s, admit over 600s. Six of eleven tracks saw tests killed by the slow timeout under load (evo 8 at once, admit 13 distinct tests), then reran each alone to show it passes: evo 117s, qual4 96s, admit 202s, eadhoc 149s. Full runs took about 72 of the 179 minutes the eleven tracks spent in cargo.

The final runs also outlast the agent shell's 120 second default and 600 second cap, so they move to the background and agents poll for them (front2 waited 763s in loops, ref3 292s, lang5 191s), and four agents tried a long `sleep`, which the harness blocks.

Fix: a devenv script `suite` that takes a machine-wide lock (`flock` on a file under `$HOME/.cache`) around `cargo nextest run --workspace --no-fail-fast`, writes the full output to a file in the workspace's `target/`, and prints only the FAIL, TIMEOUT and Summary lines and the output path. Print a line when it waits for the lock so the caller knows it queued. The build half (`--no-run`) stays outside the lock. Then the brief's full check names `suite` and says to start it with run_in_background and wait for it with Monitor.
