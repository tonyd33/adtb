---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 80
summary: "parallel agents' full test runs contend for the cores, so a 40 second suite takes 4 to 10 minutes and times tests out"
closed: 2026-10-26
tags:
  - tooling
  - devex
---

# Feature: one full test run at a time across workspaces

## Report

Agents run the full suite in parallel jj workspaces on one 16-core machine. Load during the 2026-10-26 tracks had a median of 22 and peaked at 70. The default run takes 32 to 86 seconds alone (test-suite-time), but each track's final run took 236 to 600 seconds: sound 236s, qual4 306s, eadhoc 504s, evo 539s, strength 426s, admit over 600s. Six of eleven tracks saw tests killed by the slow timeout under load (evo 8 at once, admit 13 distinct tests), then reran each alone to show it passes: evo 117s, qual4 96s, admit 202s, eadhoc 149s. Full runs took about 72 of the 179 minutes the eleven tracks spent in cargo.

The final runs also outlast the agent shell's 120 second default and 600 second cap, so they move to the background and agents poll for them (front2 waited 763s in loops, ref3 292s, lang5 191s), and four agents tried a long `sleep`, which the harness blocks.

Fix: a devenv script `suite` that takes a machine-wide lock (`flock` on a file under `$HOME/.cache`) around `cargo nextest run --workspace --no-fail-fast`, writes the full output to a file in the workspace's `target/`, and prints only the FAIL, TIMEOUT and Summary lines and the output path. Print a line when it waits for the lock so the caller knows it queued. The build half (`--no-run`) stays outside the lock. Then the brief's full check names `suite` and says to start it with run_in_background and wait for it with Monitor.

## 2026-10-26: done

`work/tools/suite [ARGS]` (devenv `suite`) checks free space, builds with `cargo nextest run --workspace --no-run ARGS` outside the lock, then takes `flock` on `$HOME/.cache/adtb/suite.lock` (machine-wide, so every workspace queues on it) around `cargo nextest run --workspace --no-fail-fast ARGS`. It prints `waiting for the suite lock, held by WORKSPACE pid N since T` when it queues, writes the whole output to `target/suite.txt` (`ADTB_SUITE_OUT` overrides) ending in `suite: exit N`, and prints only the FAIL, SIGABRT, SIGSEGV, SIGKILL and TIMEOUT lines, the summary and the path. Two runs started together: the second built in 2s, waited 13s, then ran the full suite in 57s. `check` runs the suite through it. Open: the agents' brief must switch to it; runs that do not use the script still contend.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the devex track.
