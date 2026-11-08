---
created-at: 2026-10-29
status: in-progress
kind: feature
priority: 720
summary: "no dump, restore, online backup or point-in-time restore"
---

# Feature: Dump, restore, online backup and point-in-time restore

## Request

Milestone 9, interop.md 6 and 7: dump, restore, online backup, point-in-time restore. A restore keeps the file schema-owner role (interop.md 6.1). A family grant target carries its signature, so a dump round trips it.
## Proposal

## Open questions

## 2026-10-31: S4 full backup, scoped and ruled

Rulings adopted: repl/backup-heal/new-file, restore-point/one-reason, backup-proof/separate-proof, backup-target/paths, backup-damaged/named-part, backup-files/write-files, restore-epoch/bump, backup-quarantined/heap-yes-file-no, proof-host-fn/unproven, backup-occupied/refuse. interop.md 7, catalog.md 9.1 and 11.2, transactions.md 8 state them.

Design: flush and checkpoint, keep its LSN as ck. Copy pages 2 to file_pages with read_disk; a corrupt read retries under an exclusive latch, as verify's check_page does; still corrupt is listed damaged. Force the log, copy segments from batch_start(redo) to durable(). The backup's header names ck, never a later checkpoint from the file: a later checkpoint's redo would skip repair of pages copied before it. Directory layout: pages/ records of pid and page, log/ with live segment names, manifest last and forced. Manifest: kind, format, release, edition, page format range, schema version and hash, catalog version, file id, sizes, encryption list, start, checkpoint and end positions, end time, epoch history, parent hash, parts with blake3, damaged pids, quarantined objects. Dump (S5) shares the envelope, and archive (S6) the page records, log names, parent and epoch history. Nothing recycles log segments today, so backup needs no pin; a recycling track must honour running backups. Backup runs the first checkpoint under live writers.

The S4 track makes these catalog.md edits with its code, since spec_corpus and server_commands compare them with the code: in `RejectReason`, after `DumpDamaged`, `BackupDamaged { part: Text }` and `RestorePointUnavailable { oldest: Position, newest: Position }`, each with a row in the reason table and ids; the `backup` row of 9.1 needs `maintain`, and `write_files` through a server.

## 2026-10-31: next step

S4 is in round 4, change yxnwxzvu, which replaces name-based cleanup by building aside and publishing last (`Op::Rename`). Then: delta review, rebase, replace the `p.issuer` check in api/backup.rs `backup_to` with the Disk export roots that roles-adhoc landed, plus a target-inside-roots test, full check, land. S5 to S7 follow, after fault-matrix.

## 2026-10-31: tickets from round 4, after landing

`write_pages` trusts a resealed part's chunk header, which can panic or write past `file_pages`; `restore` refuses servers. `restored` and `prove_in` hash every part twice. A failed read of the test database writes a `Proved::Failed` over a true proof; `a_failed_read_of_the_test_database_records_no_verdict` holds the spec, and fault-matrix's typed cause fixes it.

From round 6, not fixed: a misnamed foreign segment, or one of another block size, reads Invalid and is ignored in recovery, so `log.rs:350` removes it; it needs hand-placed files. `list_race_panics` survives default and deep, so no test reaches the `NotFound` arm at claim's List. No test covers races or a restore interleaving on a real file.

From round 7, not fixed: `Unformatted` has no test, since a host page fault cannot be planted in a few lines. A failed log directory removal with a successful file removal leaves a log with no file, and the next create answers corrupt. The lock's postcondition needs every renamer or unlinker to hold it; restore's final rename against an opener of that name is not checked. Two-process races are tested only through a seam between open and lock.

## 2026-10-31: S4 landed

Full backup, restore and prove build aside and publish last. A log belongs to its file by `file_id` in every block. A failed open removes only what it made, and a lock is held only while the path names the locked inode. Seven rounds. The search harness is `crates/adtb-harness/src/search.rs`. Open: the gaps listed above, an ignored red test `a_failed_read_of_the_test_database_records_no_verdict`, and two restores of one backup sharing a file id. S5 to S7 follow fault-matrix.

## 2026-11-03: S5.1 landed

`feat: dump a database to a directory and restore it`. `dump to` and `adtb dump FILE TARGET` write a sealed directory: manifest, versioned log, `ids`, generators and one text data part per constructor relation, read at the command's one snapshot, built aside and published last. `dump` in a transaction that has written is refused with `Invalid`. `restore` of a dump checks the seal, then dispatches on the kind line, checks every part and the exact part set, compares edition, schema version and schema hash (another release and an older catalog version restore), answers `RestorePointUnavailable` for any point but the snapshot, replays the log into a private file, checks parsed `ids`, inserts every row under every constraint (a deprecated constructor's rows admitted on that private file only), resumes generators, gives a new file id and an epoch past the snapshot, and publishes last. A refused restore leaves nothing at the target and removes only directories it created. Spec: `DumpDamaged { part }` replaces the byte offset; interop.md 6.1 and 6.2 state the transaction rule, the `ids` check, the restore point, the file id and epoch, and the undetermined outcome of `Fatal { Storage }` at the final sync. Tests: crates/adtb-sim/tests/dump.rs (round trip over 6 conformance scripts by default and all under `deep`, a manifest edit search, every manifest byte flipped, failure and crash searches over dump and restore), a shell test. Two rounds; review found four blockers (dump saw its transaction's uncommitted rows; two manifest panics; another release refused as damaged), all fixed with tests. Full check 3675 passed.

Lead rulings, interim: the data form is text only, though 6.1 says a restore reads the Arrow form (S5.5); a dump holds no deployment state, blobs or audit (S5.2); `restore` as a command passes no host functions, so a schema needing them refuses with `MissingFunction`.

Left: every entry is held whole in memory and restore inserts one change set, filed as dump-bounded-memory; the failure search skips renames out of `*.creating/`, filed as sim-rename-into-child. Next: S5.2 deployment state, S5.3 offline validation, S5.4 through a server, S5.5 Arrow, S5.6 encryption (needs a dependency ruling), then S6 and S7.

## 2026-11-04: S5.2 change 1 landed

`feat: carry security, settings and audit rows in a dump`. A dump carries `security` (roles, credential verifiers, `Deployment` grants, peer mappings), `settings` and `audit` entries, and the id counters of roles, grants and audit in `generators`. Restore parses every part before the aside file exists and refuses a damaged one naming it; writes security before replaying the log, so log grants to restored roles come back active; keeps each log grant's id by pairing it with the dumped row (`ids` carries the log grant rows), refusing naming `ids` on any unpaired row or a dormancy mismatch; keeps a grant dormant in the source dormant by replaying it held (DORMANT marks, `CreateOptions.replay`); restores a dump without `security` with the owner alone and log grants dormant. The dormant list is `Restored.dormant` from `restore_report_in`, doc hidden until the spec states the report (restore-dormant-report). Review found one blocker, a dormant grant coming back active, fixed with a round trip test. Full check 3388 passed.

Interim rulings: a restored role's credential is shown only by its verifier rows surviving, since the engine has no authentication to try it with; every part is parsed against the current engine schema and mapped onto the restored file's relations by name, so a catalog relation renamed across releases would need that mapping changed (untested today, mutants R6 and R7 survive for that reason). Ticketed: audit-ledger, blobs on install-package-bytes, restore-dormant-report.

Next: S5.2 change 2, `bundles/` with registration marks, family owners and expired-lease latches; `renewed` kept as an absolute instant and an expired lease stays expired (interim ruling, spec sentence in interop.md 6.2 with the change).

## 2026-11-04: S5.2 change 2 landed

`feat: carry registered bundles and their leases in a dump`. One `bundles/NAME` entry per bundle (registration mark, `bundles`, `registered`, `feeds` and `feed_members` rows) and a `bundles/index` entry (bundle names, family owners, expired-lease latches). The dump reads marks, owners and latches from the roots heap at its own snapshot (`Deployment::at`), not the engine's in-memory copy. Restore parses every bundle part before the aside file exists and refuses naming the part on a decode failure, a missing or unlisted entry, rows of another bundle, a duplicate or dangling index line, or a mark or family owner naming a role the dump does not carry (interim ruling, restore-missing-registrar); a restore registers none of the bundles passed to it. interop.md 6.2: a lease's last renewal is kept as the same instant, so an expired lease stays expired. Review found three blockers (a lease latched in the dumping session dropped, a registration after the snapshot leaking its family owner, a restore whose file fails its next open), all fixed with tests. Full check 3391 passed.

Left: `deployed.expired` is now kept current by `latch_expired` but has no reader, so that fix is unobservable; the engine has no feed writer, so feed rows in the tests are written directly. Next: S5.3 offline validation, S5.4 through a server, S5.5 Arrow, S5.6 encryption (needs a dependency ruling), then S6 and S7.

## 2026-11-04: S5.3 offline validation

`feat: validate a restored database once after writing its rows unchecked`. interop.md §6.3: `fill` writes each data part's rows into the heaps unchecked (`Restoring`, write/restore.rs, reachable only through `admit_restored_rows`, held by a private constructor and a source search test), builds every index of every relation from sorted runs checking uniqueness, checks references and domains of every row, then evaluates every constraint and check once over the whole database with the change empty (`inserted`, `deleted` empty, `prior(R)` read as `R`, constraints.md §6.2). A violation fails with the one of least name and the aside file is discarded. A generated sequence advances past every restored key. Tests: dump_validate.rs, an 11-case sweep (exclusivity, duplicate key, unique, reference, domain, row check, constraint query, least name, a mixed transition constraint, a prior-only constraint, an oversized key), each asserting the exact reason, an unchanged device and the target opening as missing; restored indexes enforce later writes; totality as `DumpDamaged`; a fault search over a violating restore; the generator resuming past restored keys; 17 mutants killed. Review found a clean dump with any transition constraint panicked (the check read the arena after the delta compiler rewrote it) and the sequence heal dropped; both fixed. Full check 3348 passed. Memory: one part's rows held at a time instead of all.

Left: migration-skips-transition-constraints, restore-violation-reports-wording. Next: S5.4 through a server, S5.5 Arrow, S5.6 encryption (needs a dependency ruling), then S6 and S7.

## 2026-11-05: S5.4 dump through a server

`feat: dump through a server`. `dump to` through a server needs `maintain` and `write_files` (as `backup`), and read on every relation whose rows the dump holds: what `print schema` needs (`SCHEMA_RELATIONS`, one list shared with `Holds`) plus the user relations, catalog::settings, catalog::audit and the bundle relations; a refusal has the existing `Holds::lacking` shape and is audited. The `security` entry, and the `roles` and `grants` counters in `generators`, are written only when the issuer holds the `schema` capability; a restore of a dump without them restores `owner` alone with other roles' log grants dormant, and draws later role and grant keys past every restored key. The path is confined to export roots by one `Engine::confine` shared with backup. interop.md §6.1 names the relations. Tests: dump_server.rs (each authority missing, each schema relation revoked in turn, each catalog relation revoked, restore with and without `schema`, path refusals, a symlink out of the roots, `encrypt` and quarantine refused as in process); 16 mutants killed. Review found the first read set let a role without schema reads dump the operator log and the grants; fixed. `check --changed` 3355 passed.

Rulings (lead): "read on every relation dumped" covers every relation whose rows an entry holds, catalog ones included; `security` is gated by `schema` alone.

Left: export-path-toctou, quarantined-file-refuses-dump. Next: S5.5 Arrow, S5.6 encryption (needs a dependency ruling), then S6 and S7.

## 2026-11-05: S5.5a Arrow IPC stream

`feat: write and read engine batches as an Arrow IPC stream`. `crates/adtb/src/batch/ipc/`: a caller-built `FileSchema` (field and constructor names, Bool, the library Option, enums) and `write(file, batches, out)` / `read(bytes, file, &mut Budget)` in interop.md §2.2's mapping, with an engine flatbuffer writer and bounds-checked reader; no production dependency. arrow-rs 60.1.0 is a dev-dependency of adtb-sim only, the test oracle (deny.toml entries). Tests in adtb-sim arrow_ipc.rs: round trips both ways through arrow-rs for every type and shape and a generated sweep, each read at its exact row budget; another schema refused naming the first difference; fieldless types at any length; padded nodes read nothing past what rows refer to; values outside their domain refused naming column and row; union children read by name; dictionaries charged once; cut and flipped streams refused or read in domain. Mutants killed.

Lead rulings (in interop.md §2.2 and §2.3): a sum past 127 constructors is a union of unions, intermediate children named by decimal index; `Option (Option T)` and `Option` of a union-laid sum are a `None | Some` union; an enum's dictionary is every constructor name in declaration order, sent once; a one-constructor value sum is a struct; `SortKey` is `binary`; `decimal128(38, s)`; fields are non-nullable but an `Option`'s present column; a reader matches constructors by name, and a writer adds no other metadata. The reader accepts any field's nullability and refuses a null outside an `Option` per value; text that is not NFC is refused, not normalised.

Reviews found: the reader read the `nodes`/`buffers` vectors of 16-byte structs as scalars; an 8x-bytes length bound refused fieldless types; hostile streams materialised hundreds of millions of rows; values outside their domain decoded; the datum cap undercounted union rows; dictionaries were resolved per batch uncharged. All fixed. Size overran the brief's stop lines twice (production 1663 lines, tests 1170); accepted.

Left: arrow-reader-lenient-forms. Next: S5.5b (dump writes `data/R.C.arrows`, restore reads it, `FileSchema` built from the catalog), S5.6 encryption, then S6 and S7.

## 2026-11-05: S5.5b dump rows as Arrow

`feat: dump rows as Arrow IPC streams and restore from them`. A dump writes every constructor relation as `data/R.C.arrows` (the S5.5a format) beside the review text form; a restore reads only the Arrow form, through the unchanged unchecked write and single validation. `catalog::arrows::file_schema` builds the `FileSchema` from the catalog, Bool and Option recognised by the prelude type's id. `write_rows` cuts record batches before any column's offsets could pass i32 (`BATCH_LIMIT`). A restore's row budget is `8 * len / w`, w the fewest bits one row can take (one row when w is 0). A missing, damaged, cut or wrong-shape stream fails `DumpDamaged` naming the entry. `DUMP_FORMAT` is 2: a text-only dump is refused as `UnknownFormat`. The per-entry pins and relation check of the text form are gone; the manifest checksum binds a stream to its entry. Tests: dump_arrow.rs (every type round trips; arrow-rs reads each entry to the source rows; look-alike user sums of Bool and Option; damaged entries; the text entry edited or removed; relations spanning several record batches), arrows_tests.rs; mutants killed. Review found a dump of a relation over 2 GiB of text panicked in the writer and the row budget admitted about 8 zero-width rows per byte; fixed. `check --changed` 3390 passed.

Left: dump-bounded-memory (peak memory now roughly doubles), quarantined-file-refuses-dump, export-path-toctou, arrow-reader-lenient-forms. A single row over 2^31 value bytes still breaks the writer's precondition. Next: S5.6 encryption (needs a dependency ruling), then S6 and S7.

## 2026-11-05 S5.6 rulings

Dispatched S5.6 (`feat: encrypt a dump under a passphrase`). Rulings: RustCrypto `chacha20poly1305` (XChaCha20-Poly1305, `aead` stream) and `argon2`, randomness from `getrandom`; no libsodium or ring. Argon2id at m = 65536 KiB, t = 3, p = 4 with a random 16-byte salt per dump, both in the manifest; a restore refuses m above 262144 KiB, t above 10 or p above 16 as `DumpDamaged { part: "manifest" }`, so a crafted manifest cannot spend unbounded memory or time. argon2 built at opt-level 3 in the dev profile. Each sealed entry is a STREAM (BE32) of 64 KiB chunks with a random nonce prefix per entry and the entry's path as associated data, so a swapped, cut or reordered entry fails to authenticate. The manifest's version fields stay plaintext and the rest is sealed under the name `manifest`. The passphrase reaches no log, audit row, error, manifest or file.

## 2026-11-05: S5.6 encrypted dump landed

`feat: encrypt a dump under a passphrase`. `dump to "p" encrypt k` and `adtb dump --encrypt KEY` seal a dump; `restore from "s" to "p" decrypt k` and `adtb restore --decrypt KEY` restore it. Argon2id (m 65536 KiB, t 3, p 4, a random 16-byte salt) derives the key; a restore refuses parameters past m 262144 KiB, t 10 or p 16 as `DumpDamaged { part: "manifest" }` before deriving. Each entry is an XChaCha20-Poly1305 STREAM (BE32) of 64 KiB chunks with a random nonce prefix and its path as associated data; the manifest keeps its version fields, `encrypted 1`, `kdf` and `salt` clear and seals the rest with the clear part as associated data; part hashes cover the ciphertext. Randomness comes from the runtime's `Entropy::fill` (the OS CSPRNG on the host, seeded in the simulator). Rulings: spec/restore-key/decrypt-clause-invalid (a missing or unneeded passphrase refuses `Invalid` before writing), security/dump-key-audit/param-only-omitted (the operand is a declared parameter, never text; a Call row's `args` omits it, and records only parameter names when the text does not parse). `Passphrase` zeroizes and its Debug prints only `Passphrase(..)`. Spec: catalog.md, interop.md §6.1, security.md §9.5 and the `args` paragraph, syntax.md; tree-sitter has the clause. Entry names and sizes stay visible, as the spec now says.

Review found an unparsed text auditing a fast hash of the passphrase, and limit tests that passed without the limit check; both fixed. 18 mutants killed. `check --changed` 3417 passed, campaign 48.

Left: cli-passphrase-argv, dump-entry-names-clear, registered-args-encoding; the per-entry decrypt failure has no end-to-end test (reachable only with the key). Next: S6, then S7.

## 2026-11-05: S6a continuous log archive landed

`feat: archive the log continuously` (storage/archive.rs). Complete durable segments go to `DEST/<file id>/<epoch>/<segment>-<blake3>` in order, gapless, each written aside, fsynced, renamed, directory fsynced; an epoch file naming the parent (only when this log continues it) is published first and never replaced. `archived_through()` is the gapless run from the file's own log start (new header field `log_start`: 0 for a new or dump-loaded file, the backup's end for a restored backup), capped at the durable end; the first pass after open hash-compares every published segment the log still holds, and a mismatch or two names for one base answer Damaged. A missing destination root, a local gap and a failed copy are recorded in the status and retried; nothing is dropped. One daemon thread; close joins it and runs a final pass that cannot fail the close. Options: `log_archive`; doc-hidden `CreateOptions.segment_size` (refused, not asserted, when off the log block). Review found two blockers, both fixed: another lineage's segments accepted under one id and epoch, and a dump-restored file claiming segments it never archived.

Residuals: within one process the comparison cache never resets, so a name deleted and republished with other bytes below it is not rechecked; the first pass re-reads every retained published segment; `OnceLock::get_or_init` around a sim lock would deadlock under a second concurrent caller (none today). The recycler's archive hook (readin-gate track) is wired to `archived_through()` when that track lands.
