---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 62
summary: the prescribed sibling jj workspaces fall outside this Codex session writable roots
closed: 2026-10-28
tags:
  - tooling
  - devex
---

# Prescribed agent workspaces fall outside the writable sandbox

## Report

`.agents/skills/lead/SKILL.md:16-17` prescribes `jj workspace add --name NAME ../adtb-NAME`. The audited Codex session permits writes under `../adtb` and `/tmp`, so sibling working copies are outside its writable roots. Creating or editing them requires additional authorization or provisioning; spawning an agent does not itself establish those permissions.

Document how the lead provisions a workspace path and grants the worker access before dispatch. Respect the active harness permissions rather than assuming all sibling paths are writable. Keep isolated working copies and per-workspace build artifacts. Validate the workflow under a restricted Codex session, including jj metadata access, without assuming unrestricted host access.

## 2026-10-28: dispatch precondition documented

Lead guidance now requires confirming worker access to both its workspace and
shared jj metadata before dispatch. Creation does not grant permission;
required approval/provisioning precedes implementation. Established permission
wait is reported separately from execution, without bypassing approval.

Still open: validation under a restricted Codex worker has not run. Pi's host
access does not establish Codex sandbox behavior.

## 2026-10-28: resolved

Validated under the current managed Codex workspace-write sandbox. Lead created
sibling workspaces and explicitly ran target write/read/remove plus shared jj
access under require_escalated before delegation. Campaign/manual/checker and
independent reviewers repeated probes successfully with the same approved
mechanism. Unprivileged reviewer shared metadata access was refused; escalation
succeeded. No permission bypass or unrestricted-Pi assumption. This verifies
current restricted Codex setup, not every future harness configuration. Approval
wait durations were not measured. Lead/code-agent guidance already requires
provisioning before implementation. Source/test verification receipts remain
separate. Original local ticket was untracked despite tracked README reference;
lead explicitly tracked it and four related resolved guidance tickets after
fresh quality workspace exposed broken links. Fresh ticket check follows.

## 2026-10-28: resolved
