---
created-at: 2026-11-18
status: resolved
closed: 2026-11-18
kind: feature
priority: 22
tags: [tooling]
summary: "one snapshot runner for conformance files and engine scripts, replacing conf, script and adtb-probe"
---

# Feature: Snapshot runner

## Request

Ruled by the maintainer in the tooling review of 2026-11-17 (checks CI needs live in the public repo, held to a much higher standard than work/). The brief is work/briefs/brief-snapshot-runner-r1.md.

## 2026-11-18: resolved

Landed in `refactor!: replace conf, script and adtb-probe with one snapshot runner in adtb-conformance`. `cargo conformance [--file-name PATH] [--include P] [--jobs N] [--fail-fast] [--update] [--reference|--engine]`, `probe` subcommand. Case counts equal (257, 9, 487, 69). The `scripts` feature alone enables adtb/testing, so the packaged adtb binary has no testing. `--update --engine` on conformance is refused. Refusal reasons and plans are not updated: runner-update-errors. The sim_test_opens structure rule no longer covers the interpreter open helper now in src/.
