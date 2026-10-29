# Registered admission handoff
Unlanded workspace `/home/tony/code/adtb-registered-admission` (`registered-admission`).
Frozen `a5b883281e07474bc8373dc345d2b9c20deabb09`, base `39dc440ef0e837f6774fe441e419c2707244a397`.
`crates/adtb/src/api/registered.rs:683-851`: shared admission, all Entry.types validation at Checked.schema, deferred renewal hooks.
`crates/adtb/src/api/mod.rs:663-704`: native options checks before the once-only readiness hook.
`crates/adtb/src/api/txn.rs:190-227`: state/merged-stop checks and renewal under run's single open guard.
`crates/adtb-sim/tests/registered.rs:777-1078`: argument boundaries, unused/imported parameters, pending-write preservation.
`crates/adtb-sim/tests/bundles.rs:1310-1714`: exact renewal timestamps, stops, ended/closed transactions and positive controls.
Retained failing baseline ids/logs and input manifests: [receipt](receipt-registered-admission.md).
Frozen focused gates pass: 173 public tests, 4 value unit tests, strict affected clippy, fmt, house, size.
Single full branch check exit 0: root 2,426 passed/8 skipped; campaign 47 passed/1 skipped; docs/policy/size passed.
No spec/design/dependency/API export changes; no new general engine/spec gap found.
No commits, ticket edits or landing performed. Lead owns fresh review, tickets, landing and full post-land main check.
