---
created-at: 2026-11-05
status: open
kind: bug
priority: 470
summary: "adtb dump --encrypt KEY and adtb restore --decrypt KEY take the passphrase on argv, visible to other local users in /proc/PID/cmdline"
---

# The CLI takes a dump passphrase on its command line

## Report

## 2026-11-05

From the S5.6 review. Process arguments are readable by other users on the host. Direction: read the passphrase from a named environment variable, a file, or a prompt on stdin, and keep the argv form only if the spec wants it; the spec (interop.md §6, security.md §9.5) names `--encrypt KEY`, so this is a spec change through the design space. The clap String and the Val::Text copies are not zeroized.
