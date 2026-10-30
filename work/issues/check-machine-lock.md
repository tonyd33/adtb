---
created-at: 2026-10-28
status: resolved
kind: bug
priority: 50
summary: "check builds and lints without a machine-wide lock, so two concurrent checks can exhaust the machine"
closed: 2026-10-30
tags:
  - test
---

# check takes no machine-wide lock

## Report

`work/tools/suite` holds a machine-wide flock for the test run, but `check` (devenv.nix scripts.check) runs clippy, the suite, the campaign build and the docs build outside any lock. On 2026-10-28 two full checks ran at once in two workspaces and the machine became unresponsive and had to be rebooted. The previous boot log also shows correctable PCIe BadTLP errors on the root SSD port 0000:00:01.1, so the load is not proven to be the only cause.

Fix: take the same lock as suite for the whole of check, printing who holds it while waiting, as suite does.

## 2026-10-30: landed

`check` and `suite` hold one machine lock, taken in work/tools/common.bash. Its gaps are on tool-lock-gaps.

## 2026-10-30: resolved
