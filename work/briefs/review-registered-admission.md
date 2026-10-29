# Independent registered admission review

Frozen candidate `a5b883281e07474bc8373dc345d2b9c20deabb09` on
`39dc440ef0e837f6774fe441e419c2707244a397`; reviewer workspace
`/home/tony/code/adtb-registered-admission-review`, empty `1de33baf` on candidate.
Read-only review of all five changed code/test paths; no Rust build or test,
source/ticket edit, spec change, dependency change or commit by this reviewer.

No concrete source blocker found. The exact frozen candidate is clear for the
lead to land: the full branch check exited 0 and all frozen inputs remain
unchanged. The lead owns landing and the full post-land main integration check.

## Responsibility and ordering

`api/registered.rs:730-758` keeps registration-specific checks together:
ordinal, native lifecycle decision, current pin, caller, argument count and
representation, and interactive read-only mode. Existing refusal meanings and
their order remain; argument representation validation is the new step after
count. `arguments:798-817` calls the existing `api/value.rs::valid` against
`Checked.schema` and all `Entry.types` before preparing/cloning arguments.
There is no second validator, parser/printer path or transport-only policy.

The parallel vectors have complete positional coverage. Query and transaction
checking binds every declared parameter in source order, wraps every binding
in a lambda, and `norm::registered` captures those leading lambdas before
normalizing the body. `free_var` preserves each variable's type. Their Entry
producer maps every normal parameter to Some and derives types from those same
positions. Zero declared arguments produce empty Entry params/types and an
internal unit argument. Imported functions derive both vectors from every
declared function parameter, retaining a None position when normalization has
eliminated its variable; types are still present. The validator's zip therefore
covers every declared argument under the current producers.

Standalone and Arrow share `Registered::call:683-725`; interactive calls share
the same admitted helper. Renewal eligibility then reaches the existing native
execution boundary through a narrow FnOnce. `Engine::call:663-702` checks stops
and the bookmark before invoking it, once before the retry loop. Interactive
`AdHocTransaction::run:190-224` checks its existing open guard, ended state and
merged call/transaction stops before invoking it. Ad hoc callers supply an empty
callback. This reuses the native readiness checks without extracting a second
readiness policy or adding a separate execution entry path.

Standalone keeps its existing one outer engine guard through validation,
renewal and execution. Interactive `run` owns its existing one guard around
renewal and execution; the earlier admitted helper reads clock/settings,
deployment metadata and immutable checked data, and does not renew or access
closed storage. No extra nested engine read guard was added. `Engine::renew`
still uses the native try-lock, rechecks lifecycle/due state, and ignores a failed
renewal while preserving the stored lease. Serialization retries and transaction
abort ownership are unchanged.

This hook establishes renewal eligibility once. It does not provide future
per-retry role authorization, serialize a lifecycle change with execution, or
make cancellation/deadline expiry atomic with renewal. A token cancelled or a
deadline reached after initial readiness can still cause an execution failure
after renewal. Those are existing bounded-admission limits, not claims that
every runtime failure leaves a lease unchanged.

Representation validation preserves write-only newtype predicates, empty/short
Uuid Bytes values, and keys without referent-existence checks. Untagged Value
cannot prove nominal identity, and the patch does not invent such a check.

## Regression and verification evidence

The complete diff adds 12 focused tests without weakening existing tests.
`registered.rs:777-1078` checks an unused I8 write parameter, no invalid write,
pending-write preservation and usable transaction after refusal; imported unused
parameters, scalar endpoints, generic sum fields, exact nested arity, constructor
sets, keys, sets, Unicode, dates/time/decimal and unit are exercised through
standalone, Arrow and interactive calls. Valid domain-violating values remain
callable, while a separate stored-value control still fails Violated.

`bundles.rs:1310-1712` compares exact catalog renewed timestamps for bad count,
ordinal, asserted principal, value, read-only write, cancelled token, expired call
or transaction deadline, invalid bookmark and ended transaction. Positive calls
renew the same handle, refusals preserve transactions where their existing
contract does, and existing lifecycle/earlier-pin controls remain present.
Added tests took 0.426-0.656 seconds each in the final focused run.

I read the actual retained baseline logs and source revisions. At `ca493bf0`,
the unused I8=128 write returned Committed Int(1); wrong arity advanced renewal
from `2026-01-01T00:00:00.135841Z` to `2026-01-01T04:00:00.147272Z` (exit 100).
At `9ac79482`, the cancelled/deadline/bookmark/interactive-stop/transaction-deadline
failures changed renewal; the original ended fixture failed its result-count
assertion and is not an ended-state witness. Corrected `daeee04d` returned the
original Serialization error but renewed `00:00:00.135854Z` to
`04:00:00.149982Z` (exit 100). Closed-database control already passed in the
pre-hook baseline; no reproduced close panic is claimed.

Author receipt: `work/briefs/receipt-registered-admission.md`, with exact commands,
toolchain and retained logs under `/home/tony/code/adtb-registered-admission/target/`.
I inspected final nextest summaries: 173 public-path tests passed across
registered/bundles/principal/embedding/api; four shared-validator unit tests
passed. Strict affected clippy, fmt, house and size logs passed, and the focused
gate exit file is 0. These are inspected author executions, not reviewer-run Rust.

I independently hashed every one of the 2,568 entries in
`target/admission-frozen-inputs.sha256`: both author and reviewer workspaces
matched, with no missing or changed input. This includes tracked configuration,
manifests, lockfiles and toolchains. I also checked differing historical manifest
entries against all three named baseline jj revisions; all matched. Before/after
focused/final-gate ids are the exact candidate, with only the five intended paths.

Reviewer verification: actual escalated target write/read/remove and shared jj
status succeeded. `work/tools/house-check . --from 39dc440ef0e837f6774fe441e419c2707244a397 --to a5b883281e07474bc8373dc345d2b9c20deabb09`
passed, exit 0, no output. Reviewer jj status remained clean on empty `1de33baf`.
Permission, execution-only and queue timings are unknown.

I independently inspected the collected full-check exit file (0), both full
before/after ids (the exact candidate), final status (only the five intended
paths), root suite exit 0 with 2,426 passed and eight skipped, campaign summary
with 47 passed and one skipped, and completed policy/static/docs output. The
full command was run from the author workspace:

```sh
direnv exec /home/tony/code/adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check >target/admission-fullcheck.log 2>&1
```

The wrapper records `2026-10-29T00:35:12Z` through `00:42:10Z`, 418 seconds
including environment setup/queue/execution. After that completion I rehashed
all 2,568 frozen inputs in both workspaces: zero missing files or mismatches.
The frozen manifest SHA256 is
`bc44c9ea6ccd7002b710ac620f545ac74a370ac9f7466e126bee8abf105c6c84`.
No required branch check remains incomplete. This receipt does not certify a
changed combined tree after landing; the lead's integration check remains.
