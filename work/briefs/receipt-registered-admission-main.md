# Registered admission main verification

Landed `744bddb0592a6a3fdbf01bca00e2f02149b0c6c1`. Full main checked clean
`4b5eec71ca4a1631e2d965172e3e471841b33c2c`, same revision before and after.
Reviewed `a5b88328` differs only in ten ticket/evidence metadata paths; all five
code/test source files are identical. A fresh exact-source reviewer cleared it.

From `/home/tony/code/adtb`:

    bash target/registered-admission-integration.sh

Underlying command:

    direnv exec /home/tony/code/adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target check

Session 14135 collected exit 0. From 2026-10-29T00:44:58Z to 00:49:08Z:
250 seconds, one-second precision. Permission, queue and execution-only timings
are unknown. All tracked and local root/app Cargo inputs and revision stayed
unchanged. Rust 1.98.1/Cargo 1.98.0; configured profiles/features, own main target.

All 2,426 root tests passed, eight skipped (49.447 seconds). All 47 campaign tests
passed, one skipped (20.290 seconds). Root/app lint, formatting, parity, dependency
policy, size and docs checks passed. Docs: ten tests in 10.255 seconds, 420 syntax
inputs, 61 pages, 6,734 links, 4,999 fragments, zero failures.
Evidence in `target/registered-admission-integration/` includes log, status, suite
copy, start/end, revision/input snapshots and an empty comparison result.
Log SHA256: `627e1dec5b204ef4c6a9b12f655ad42e1a0f0d585e4d12e4cfe7594862cb30bb`.

Branch full check on `a5b88328`/base `39dc440e` collected exit 0 in 418 seconds.
All 2,568 tracked input hashes matched before/after and independently in both
workspaces. Baselines `ca493bf0`, `9ac79482`, `daeee04d` and fixture caveats remain
in the tracked branch receipt/review. No failed witness was erased.

After author and reviewer completed, a process audit found no users of the branch
workspace. All 796 nondebug files were archived and individually SHA-verified;
original source, logs and fixtures remain. Only completed `target/debug` was removed.
Archive: `target/registered-admission-evidence/branch-target-without-debug.tar.gz`.
SHA256: `c6b87121f3d8b4238f31599503f90f1402bc80d85e4b253228b29a3c4b0b8aba`.
Idle sequence-repair (601 files) and campaign-live (605 files) `target/tmp` trees
were also losslessly archived, with per-member SHA checks and restore commands in
`target/registered-admission-fixture-archives/`, before removing archived copies.
Unknown openperf and main active cache were untouched. About 51 GB free at finish.

The concrete bugs are resolved; broad roles/followups stay open. This receipt
certifies shared call admission, not permission execution or a server feature.
No owned agent, check or server remains active. Push stays deferred per the user.
Next: the typed-permission-requirements provider.
