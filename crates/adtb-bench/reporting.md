# Native reporting artifact version1

The application producer writes `RUN/database/application.json` last by atomic
rename, after syncing `calls.ndjson`, `attempts.ndjson` and `workflows.ndjson`.
Native service evidence lives separately under RUN. A complete application
artifact alone cannot make a run comparable; bench-report joins matching native
invocation/result/resource/context evidence and validates the complete schema.

Application fields: schema_version1, kind `adtb-bench-application`, status
`complete|incomplete|invalid`, error string or null, invocation_id string or null,
peer `adtb|sqlite`, workload `counter|counter-smoke|lifecycle`, integer n/writers,
generator_version `baseline-v1`, offered_model `finite-closed-loop-per-actor`,
build {profile,opt_level,debug,host,target} strings, settings string, window null
or {start_ns,end_ns}, plan array, record_counts {calls,attempts,workflows}, phases,
storage, validation and process_io. Build values are Cargo-provided categories
and settings, not a source revision or proof of an arbitrary binary's provenance.

The runtime size policy requires N >= 4, N divisible by 4, N dividing 20,000,
and 20,000/N >= 2. It preserves four equal campaign partitions, integral counter
visits and at least two ownership-rotating key passes, and implies N <= 10,000
and coprimality with permutation stride 37. Both workloads use this size policy;
`counter-smoke` additionally requires N=4. Writers are 1 or 4.

Plan entries contain actor/call integer identity, action, target integer key and
workflow integer key or null. Actions are `bump`, `start`, `read_live`, `finish`,
`read_played`. Actor/call identifies the fixed queue entry; target identifies a
database key. Bump has no workflow. Write calls are bump/start/finish; query calls
remain separate from write-commit counts.

Call records: {actor,call,action,target,workflow,offered_ns,ended_ns,outcome,
failure_kind,attempts}. Outcomes are completed/failed/uncertain. failure_kind is
null or retry_exhausted/budget_exhausted/refused/uncertain/fatal/output_mismatch.
A started call may exhaust its actor budget before any actual attempt. Planned
calls without a record were not started, not failed attempts. Successful latency
is ended_ns minus offered_ns, including waits, every attempt and retry backoff.

Attempt records: {actor,call,attempt,started_ns,ended_ns,outcome}. Attempt is
one-based. Outcomes are completed/retry/refused/uncertain/fatal. A final retryable
failure is observed but is not a fictional next attempt. Native internal retry
or abort totals are not inferred from these outer observations.

Workflow records: {actor,workflow,offered_ns,ended_ns,outcome,calls}. A completed
lifecycle workflow requires all four ordered successful calls. Its interval is
first Start offer through final decoded ReadPlayed. Failed/uncertain workflows
retain only their observed partial span through the last terminal call; this is
not a completed workflow latency. Counter workflow records are empty.

All timestamps are integer nanoseconds from one process monotonic origin. The
steady window starts common release after every actor prepared and ends the last
terminal operation, excluding join/validation/reopen/report overhead. Calls are
offered one at a time by each closed-loop actor, not all at release. Nearest-rank
percentiles sort integer samples and use rank ceil(p*N); empty populations have
no percentile. Throughput is completed calls, write calls or workflows per window
second, only when complete validated application and native evidence agree.

Phase entries are {name,actor:null|integer,start_ns,end_ns}. Complete runs reach
directory_create,fixture_generation,initial_open,settings,load,loaded_validation,
actor_open,actor_run,prepare(one per actor),steady,ledger,validation,
active_inventory,close,reopen,reopened_validation,final_close,closed_inventory,
artifact_write. SQLite additionally reaches coordinator_close and post_open.
Preparation overlaps across actors; actor_run encloses preparation/operations/
join/actor-drop. Do not sum overlapping phases. artifact_write covers raw NDJSON
writes before final application manifest publication.

Storage {active,closed} arrays contain {path,logical_bytes,allocated_bytes};
allocated bytes are Unix blocks*512. Inventory includes only database engine
files, excluding ledger/report artifacts. Active means the validation connection
is open before explicit close, after actor handles dropped; SQLite may already
have checkpointed at last actor close. It is not peak or in-operation WAL size.
Closed means after final reopen validation and final close.

Validation booleans loaded,before_close,after_reopen,closed_form retain all prior
oracles. closed_form means required job coverage passed plus requests/N counter
values for counter workloads; lifecycle requires N completed workflows. Any
missing/failing gate prevents comparable success.

process_io is {status:"available",source:"/proc/self/io",boundary:
"after-final-close-before-artifact-write",counters:{rchar,wchar,syscr,syscw,
read_bytes,write_bytes,cancelled_write_bytes}}, or status unavailable and reason.
These are actual CLI process/threadgroup counters up to capture, excluding
wrapper/other processes. Storage read/write bytes are distinct from syscall
logical bytes and are not guaranteed physical-media writes.

Native resources use RUN/resources/{invocation,boundary,cgroup,cpu.stat,
memory.peak,memory.events,io.stat}; failed reads retain .error reasons. Scope is
whole-service-including-wrapper-and-stop-helper-through-capture, boundary
exec-stop-post-before-unit-unload. Missing io.stat is not zero; readable empty
io.stat means zero observed attributed device I/O. Preserve its unavailable
reason alongside the explicitly different process I/O fallback.

`fixtures/reporting/complete-counter` is a synthetic versioned schema/arithmetic
example, not an engine result or measured performance claim.
