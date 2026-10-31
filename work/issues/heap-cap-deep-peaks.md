---
created-at: 2026-10-31
status: open
kind: bug
priority: 486
summary: "Measure test heap peaks under the deep profile, where a sweep could abort at the default cap"
---

# Heap cap deep peaks

## Report

Peaks behind the `threads-required` overrides in `.config/nextest.toml` were measured at the default profile. `profile::DEEP` multiplies seeds and cases, so a sweep can pass 1.5 GiB and abort falsely. Run the deep profile once with peaks logged, and raise what needs it.
