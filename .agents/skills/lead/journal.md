# Lead journal

The lead's own record of how the leading is going, for whoever leads next. Read it when resuming and before a wide review. Add an entry at each wide review, after a mistake the maintainer had to catch, and when a bet below resolves. The repository is public: no machine details, home paths or personal data.

Keep Current bets and Habits to watch current, and edit or delete what stopped being true. Entries are a record: correct one by adding a later entry. When a lesson can become a rule, a tool or a type, move it there, and say so in an entry. A lesson that only lives here is the weakest kind.

## Current bets

Each bet says what would prove it wrong.

- Mechanisms beat instances. Bet: once a mechanism in [the defect classes](defect-classes.md) lands, fix rounds in its class fall to near zero. Compare counts at each wide review. If a class keeps producing bugs, its mechanism was wrong. Open since 2026-10-31.
- Roles-adhoc lands with separate limits, as deliberate debt. They refuse safely in process. Wrong if a socket accepts text before `one-spend-budget` bounds memory and time. Open since 2026-10-31.
- A restore raises the epoch by a random step, not a history id. Wrong if two restores of one backup ever need telling apart with certainty. Open since 2026-10-31.
- Grant identity by target text is suspect. A grant as a set over catalog ids may remove the need for most carry rulings. Decide before more carry work, after checking when ids exist during a migration. Open since 2026-10-31.

## Habits to watch

- Relaying review findings as a fix list instead of asking what allowed them.
- Ruling on the case an agent brings, with no general rule, and without searching for what else breaks it.
- Reporting an event as unexplained before reading the evidence that exists. After a disk drop, `coredumpctl list`; after a crash, `journalctl --user`.
- Counting green tests, closed tickets or landed commits as progress. Ask what a user can now do, and what can no longer go wrong.
- Saying "deep" or "done" about plans.
- Waiving a stop line or a time limit as "mostly mechanical". Four of eight tracks on 2026-11-03.
- Filling the queue with hygiene tracks because they land cleanly, while the roadmap stands still.
- Accepting an agent's premise, or my own, without a run. My "65 `or`s stay refused" was false.

## Entries

### 2026-11-01: taking back over, and an audit of the prior range

What happened: another harness led 89 commits while I was away: 63 docs, 2 that change database or language behaviour. I audited it with four read-only reviews and a full check. The check passed on 212e748f once its late stages reran with TMPDIR off a full /tmp: 3516, 13 and 48 tests, Miri, tooling. No wrong answer, lost data or lost test found, by reading. The SQLite baseline is real.

What went poorly in that range: a track grew to 6327 lines against a defect that needs a few, because a contract written to rescue one implementation choice was treated as a requirement. Three exemptions from the full check were written by the lead whose track was waiting on them.

Rule: a lead does not write itself an exemption while a track waits on it. A contract with no ruling behind it does not bind. When a fix passes about ten times its first estimate, ask again what the defect is before funding more. Test: does the next large candidate get asked this before its fourth review?

What I did: I checked the key claim in code before ruling on a read-only verdict. My earlier handoff passed on an unverified track, which started that range badly. A command of mine hung here on a `cat` with no input; write files with the editor.

### 2026-10-31: churn, and what I missed

What happened: the maintainer feared a week of one-off, shallow patches and told me I should have a wider view. A read-only review of fixes and open bugs found it half true. Language and kernel fixes mostly removed their cause. Failure paths, authority and grant carry did not, and in the last three big tracks later rounds found the same kind of bug as earlier ones. About 130 fixes and bugs came from seven causes.

What I did wrong: I passed findings along with a ruling for each and stayed inside each track. Nothing compared causes across tracks. Several rulings patched the case in hand: grant identity by target text, followed by 13 more rulings; capability names, leaked then special-cased; a run height of 64 that fits a stack and states no rule; whole-database quarantine for an unowned page. Milestone 4's exit overclaimed. We also ran many review rounds on authority logic no client can reach.

The underlying gap: the rules that keep breaking were held by habit, with nothing in the code enforcing them. Strings and positions stood in for identity, panics for errors, per-path checks for one gate.

What changed: the defect class register and five tickets, the wide view section of the lead skill, batches by cause, invariants in briefs with what enforces each, and bug reports stating cause and class.

What I don't know yet: whether any of this works. No mechanism has landed. Process rules failed this week, so the weight has to sit on types, searching tests and mutation, and the measure is the class counts.

### 2026-10-31: the test audit

The maintainer asked for a wider look after one review found tautological assertions. About 80 candidates by reading, nothing run. Core model tests (key, batch, mvcc) held up. The worst were sweeps that pass having tested nothing, the check that binds the spec's prelude to code, and tool tests `check` never runs. A green `check` meant less than I had been treating it as. A delta review of the track I just landed had not flagged a test that passes with its limit removed, because I had not asked. The skill now asks. Floors go before fault-matrix, since its sweeps would inherit the same hole.

### 2026-10-31: slot-test

The maintainer asked why slot-test looked so complex. It grew from 351 to 532 lines in a day, most of it protecting the script from itself. I took one accident, ruled an absolute guarantee bash cannot give, then patched each hole a reviewer found, and I read the result and called it right. My own skill says each wrapper needs a concrete unmet requirement, and I did not apply it. Asking for a guarantee creates machinery: ask what the accident needed, and what the smallest change is that stops it. A review that lists holes in a guarantee can mean the guarantee is wrong.

### 2026-10-31: backup rounds 5 and 6

Round 5 fixed the pairing by id, and the review then found `Database::open` deleting the same restored log on a failed create, on a path the simulator tests never use. The harness searched `open_in`, and the bug was one layer up. Same class, a sixth round. A mechanism only holds where its search reaches, so a search of a layer says nothing of the layer above: name the entry point a user calls. Also, I accepted in round 5 that a refused create leaves its file, to avoid name-based cleanup; the real rule was to remove what the call made, by a fact from the operation that made it.

### 2026-10-31: when review stops

The maintainer asked for a principled reason the cycle would end, and I had none. My exit test was a clean review, and a review of a large surface never comes back clean. Rounds 6 and 7 of backup found defects already on main. Holding the branch protected nobody. The lead skill now defines a blocker by what the diff introduces or makes reachable, and caps delta reviews. Bet: track rounds fall to about three. Wrong if landed tracks start producing the data loss the cap let through.

### 2026-10-31: reflection, end of the first day of this practice

Went well: naming causes moved backup from name-based cleanup to build aside, and from paired names to a file id. The harness found a bug in new code that review missed. Running claims corrected a review twice. Going to the code for the root of a contradiction gave the merge ruling a general rule.

Went poorly: I reported a hung track as running for 80 minutes. I gave a 20-minute estimate that assumed a clean review. I ruled absolute guarantees that became machinery, in slot-test and in a run-time child check, and replaced each within hours. I pushed once without reading first. The maintainer caught all of the largest problems: churn, slot-test, and no end to the review cycle.

Do differently: state an end condition when starting a track, not when asked. Give a range and what it assumes. Ask what an accident needed before ruling. Check that a track moved before saying it is running. Read before each push, in its own command.

Bets to check next time: whether I catch a problem of this size before the maintainer does. So far, no.

### 2026-10-31: reflection, closing the session

The review cap held on its first real use. staged-operators got one delta review, which found a real regression, an upsert overwriting a row of another constructor; the agent reproduced it, the fix got no new review, and the track landed. Without a review that bug would have shipped, so the cap did not cost the review its value. One track is not a result: the three-round bet stays open.

I wrote the test-audit-b review's blocker list too widely, adding a lint that passes a violation. The reviewer reported one by that list. It loses no data and gives no wrong answer, so by the written rule it is a ticket, and I ruled so. The prompt should quote the rule and add nothing to it.

The test-audit-b suite was stopped for low memory, and I had not looked at machine memory before starting two tracks and a check on a machine other work was also using. The heap cap and the machine lock bound what this repository's tests use, not what is free. Going forward: read free memory and swap before a full check, and do not start one when they are short. If this recurs, it belongs in `workspace-gc --guard`, next to disk, not in this journal.

### 2026-10-31: resumption and contract evidence

The review cap still holds: test-audit-b goes to verification with its review findings ticketed, not another review round. The prior handoff's claim that the branch was on main was stale by a documentation delta. I started linting before checking its parent, stopped that run, rebased, and restarted. Live ancestry must precede checks even when the handoff says it is current.

The pinned-write investigation separated an explicit contract violation from an unrun candidate. Hidden-row overwrite already contradicts the key-refusal rule, whereas the ordinary nested-field scan appears to have the required reader gate. Existing upsert tests expect faulty outcomes. The next track runs both through the public API before choosing a fix. This tests the earlier bet about mechanisms: a shared source-scope contract must cover probes as well as scans; another field-shape patch would not.

### 2026-10-31: child permissions and refusal expectations

I verified that the lead could write sibling workspaces, then treated that as proof that workers had the same approval access. Two workers stalled on their first escalated command, one for over sixteen minutes. Their files and build processes had not moved, but I let the absence of a reply stand as analysis in progress. The existing instruction requires worker access, not lead access; I did not satisfy it.

The replacement for this session uses ordinary worker reads and agent-authored patch files in the writable checkout. The lead inspects and applies those patches and runs commands through the normal approval flow. No custom watcher or runner was added. Future dispatch needs a worker-side access result before an implementation is called active; an outstanding approval is blocked work.

Two full audit runs exposed guessed refusal expectations: an incomplete list of missing grants, then a named unique mistaken for a relation key. The second depended on a host-thread branch that the earlier run could skip. The repair now derives the exact report from the fixture and owned source row and gives the same helper a deterministic invocation. Typed serialization matching replaces a debug-string test. This is why exact assertions need independent expectations and evidence that the operation ran, not merely narrower matching.

### 2026-10-31: verification closed the bounded hiding slice

The review cap held: the hidden-write fix received one independent reading review, then measured guard mutations and full verification without another broad review. Removing the engine guard reproduced the successful overwrite; removing the reference guard ran the forbidden branch; dropping the translator intersection widened the checked source. All sources were restored and the honest checks passed. This demonstrates those enforcement points, not that the remaining reader-policy defects are fixed. The public unique integration gap remains explicit rather than weakening its normative name assertion.

The test audit passed both branch and landed checks and was published. Its deterministic unique-refusal helper killed the wrong key-error validator. The root runtime pair showed no observed increase; a prior slower sample remains recorded. These are narrower claims than a complete test-quality repair or a statistically established speedup.

A stale sibling snapshot surfaced during rebase as divergence and later made land refuse. I compared both trees, restored the already verified import correction, and gave the preserved older snapshot a separate change identity. No work was abandoned. Future cross-workspace rebases must first snapshot the source workspace itself; taking the root view alone can miss unsnapshotted sibling edits.

The mechanism bet remains open: one landed hiding fix and its mutations do not establish a drop in class-wide recurrence. The next fault-matrix slice must reuse the operation search and name the public entry point, with exact outcomes and reached-case floors before adding a broad sweep.

### 2026-10-31: host-open boundary and complete refusal expectations

The first fault-matrix slice deliberately stops at the host acquisition protocol. The runtime operation search cannot reach syscalls before the runtime exists. A small private test-only seam reaches the actual public open, and the production repair uses the existing bounded retry rather than another runner, filesystem layer or retry budget. The old protocol failed after the exact post-log-claim refusal; false ownership in the opposite direction also failed preservation. Both are measured assertions, not reading claims.

I checked the strengthened preservation test's diagnostic against the language code but failed to follow its public error wrapper. Its exact assertion rejected Invalid where migration returns SchemaRefused. A fresh narrow correction traced the whole conversion and retained the exact category, operator, detail, version and acknowledged row. This repeats the earlier refusal-expectation lesson: diagnostic wording is not the complete expected outcome. Future expectation receipts must identify the public conversion as well as the diagnostic source. That remains verification work, not a compile-time guarantee.

The independent review found no introduced blocker and one preexisting acquisition cleanup candidate, now ticketed separately. The bounded review rule held: that candidate and a test expectation did not trigger another broad review or expand the fix. The class-wide mechanism bet remains open; a finite host-open search is not coverage of all engine faults.

### 2026-10-31: fatal identity before the commit search

The existing process helper accepted an ordinary owned panic string as an expected fatal-I/O crash. We measured that false acceptance before replacing the diagnostic-prefix rule with one private-construction payload carrying IoError. Restoring the shared prefix filter and a Log string producer each failed its named test. This is the type-based enforcer the mechanism bet asks for, but it does not establish all failure-path correctness.

The first review traced the prefix consumers but left the broad host callback catch as unchanged behavior. My final catcher audit found that it would erase the same genuine engine payload. A public callback fixture then measured exactly that FunctionFailed conversion, so I included a small typed propagation guard. The rule is by cause, not catch location: ordinary provider panics stay statement faults, genuine engine I/O still stops the process. The source change is bounded to that guard; no new broad review round or search framework follows it.

The worker-control test again exposed an expectation sourced from only part of a conversion path: internal Structure.at is diagnostic text, public Finding.at is time, and public Structure.where holds the text. I checked the Rust diagnostic construction without checking the catalog shape. The exact assertion caught it; the correction followed the full mapping. A read-only review also missed it, which is why its verdict remains reading evidence. This test repair is specific; generated catalog codecs remain separate work.

### 2026-10-31: reuse verification of an unchanged tree

The maintainer caught the full check running twice on identical source. The
lead workflow explicitly required both runs, and I followed it without asking
what evidence the second added. The typed fatal-I/O track passed both, but
that repetition added delay without testing a changed integration.

The lead, author and receipt instructions now assign one final full check
owner. After landing, the lead compares trees and reuses the passing receipt
when test inputs are unchanged. A changed delta gets affected checks; a new
full run needs an impact that cannot be bounded. Targeted checks remain the
iteration loop. This removes the mandated duplicate; it does not claim that
the remaining full check is fast or that all development delays are fixed.

### 2026-10-31: assess mechanism coverage and delivery cost

A fresh read-only assessment verified the three recent fixes against source.
They improve shared failure identity, creation provenance and checked/runtime
scope boundaries. None closes its whole defect class. The mechanism bet has
some implementation evidence now, but no measured recurrence trend.

Ticket history is flatter than the apparent growth: first cause review
`4ba971c9` had 165 unresolved tickets, last reviewed and current `bd61e6fc`
both have 166. The recent range added one and resolved one. Earlier declines
also included wontfix triage, so counts alone cannot establish delivery.

The lead made later beta work wait behind a fault matrix spanning every
workload, fault and backend. That is an unbounded delivery prerequisite even
though individual tracks are bounded. The next decision is a finite public
statement/commit recovery checkpoint, followed by an explicit beta gap audit
and a feature or benchmark increment. This does not close the wider matrix
or waive existing durability promises. Catalog, admission and budget work
must justify their scope by the consumer they unblock.

The fatal-I/O final check contains about ten minutes of recorded sequential
build/test/lint stages, including 215 seconds building root tests and 96
building campaign tests. Its unchanged-tree rerun spent another 265 seconds
executing tests, before other tooling. Across the last three fixes duplicate
post-land test execution totals about 801 seconds. Two named mutation runs
each compiled for roughly a minute to execute a six-millisecond regression.
Miri was 27 seconds per run. The duplicate policy is fixed; compile cost and
suite selection remain measured optimization candidates, not solved problems.
No reliable coding-versus-checking percentage exists from these stage logs.

I should have set a finite delivery checkpoint and noticed the redundant
verification before the maintainer asked. Keep the next increment bounded;
do not answer another delay by adding a universal gate or another broad audit.

### 2026-10-31: cohesive ownership and test cost

The fatal-I/O change used seven agents, including separate handoffs for a
projection, counter and tally correction. Its first production patch existed
about five minutes after the author brief, but the investigation-to-publication
receipt interval was about 45 minutes. Artifact times are approximate phase
boundaries, not a complete activity log. The source commits for the last three
fixes were about 45 and 42 minutes apart.

The maintainer approved keeping an author through corrections, overlapping
snapshot review with targeted verification, reducing administration and
measuring build feedback. Context caching was a reason for frequent agents
in another harness, not a reason to fragment this work. Subagents now use
medium reasoning; the lead retains the user's setting. Independent review
still has a fresh reader. Test value must be evaluated against runtime and
retained stronger coverage, with no quota of deletions or weaker searches.

A bounded read-only check of the recorded slowest tests found distinct
permission, stack and recovery obligations rather than justified deletions.
The weak crash-image golden group costs under half a test-second in that
recording; removing it would not explain or cure the delay. No tests were
removed and no build was run for this assessment.

### 2026-10-31: refresh worker capability before preserving a fallback

I carried the patch-only execution workaround forward from earlier approval
stalls without repeating the worker-side probe. That made the lead a serial
command launcher and log relay. The maintainer caught it. Fresh probes from
both current authors immediately succeeded at workspace writes, shared jj
metadata and command execution. Authors now own execution, result assessment
and corrections; the lead owns scheduling, evidence judgment and landing.
The existing access-preflight rule was sufficient. I needed to apply it to
the current session rather than treat historical failure as current evidence.

### 2026-10-31: postmortem of the execution-ownership incident

The initiating event was an earlier worker execution stall. I preserved a
patch-only workaround in the handoff, carried it into new briefs without
current evidence, and explicitly prohibited author execution. Authors obeyed;
their lack of execution therefore supplied no independent evidence that the
restriction was necessary. I nevertheless described inability as fact.

That converted an uncertain environmental observation into an organizational
constraint. I absorbed launching, monitoring and first failure triage, spent
lead context on delegated work, and serialized handoffs. The maintainer had
to identify both the role drift and the untested premise. Fresh probes from
both authors immediately succeeded. Earlier stalls are not disproved, but
they did not justify the current restriction. The precise share of elapsed
delivery time caused by this incident is not measured.

The existing worker-access rule already covered the immediate probe. The
broader correction is an ownership and evidence contract: authors own work
through executed verification and corrections; operational restrictions need
current observed evidence and scope; temporary exceptions have an owner and
an exit condition. Command delegation does not delegate diagnosis back to
the lead. The skill and author instructions now state those contracts. The
current handoff must separate facts, assumptions and blockers from archives.

I will assess this correction through the next tracks' receipts and delivery
stages: authors execute their checks, diagnose their failures and deliver
reviewable evidence while independent work proceeds. If I again become the
routine command/log relay, the ownership arrangement has failed and needs
resolution. This is a process control, not a type-level guarantee. No new
watcher, approval layer or test gate is warranted by this incident.

### 2026-10-31: restore a finite beta delivery sequence

The ownership correction held through the last batch: authors ran and
assessed their own targeted proofs, and one author verified both reviewed
changes in a combined final tree. Native landing preserved both commits;
only reviewed ticket prose differed, so no post-land full run repeated.
A shell-only author fixed forced workspace selection with native Cargo
semantics while that check ran, without another Rust build or parser.
These observations support the workflow change, not a defect-class closure.

The remaining full check still took about fourteen and a half minutes.
Root and campaign test builds contributed roughly eight minutes, distinct
from execution. The fixture migration establishes a large local input-edit
feedback improvement, not a clean-build or whole-suite improvement. Repeated
old/new warm switches also cost about forty-five seconds each in that
experiment. New briefs warm each form once and require the necessary
sensitivity and no-recompile proof rather than an arbitrary trial pattern.

I had also answered status and stopped with completed recommendations
unassigned. The maintainer became the scheduler. This batch continued
through review, integrated verification and publication before yielding;
new work now has explicit execution owners and a source-only milestone
assessment ran alongside it. That is a process correction to observe, not
a guarantee produced by another reminder.

The beta assessment checked current public callers against all six plan
bullets. It disproves reuse of the older 6/53 tally, but is not a new
53-requirement audit or exit claim. The finite public commit checkpoint
requested earlier in this journal has landed. The next selected milestone
increment is the existing native SQLite baseline, followed by a measured
optimization or bounded vacuum consumer. The milestone ticket now replaces
its appended all-umbrella scheduling with consumer-specific prerequisites.
Existing failure, identity, authority and budget bets remain open; there is
no measured recurrence trend that justifies calling those classes closed.

### 2026-11-01: notifications need an active lead handoff

Authors retained execution ownership: the typed reference migration arrived
with preserved outcomes, reversible assertion probes, exact restoration and
an immutable candidate. Independent review overlapped complete package
validation; the latter took 8.829 seconds including compilation. The landed
source matched the tested source; reviewed ticket prose did not trigger a
second engine-wide run. The bounded verification policy held for this slice.

I nevertheless ended the preceding lead turn while the authors were working.
Their completion reports queued until the next active lead turn, leaving a
ready candidate and an approval rejection without a lead decision. The
maintainer prompted resumption and challenged my description of completed
tracks as active. This repeats the earlier scheduling failure; the ownership
correction alone did not resolve lead follow-through. Keep the lead active
through a dispatched batch's report, ruling and review or explicit blocker
handoff, and describe observed state instead of an old assignment. This
remains a process obligation, not an enforced automation guarantee.

The pinned-reader experiment also shows why green authority tests are not
reader precision evidence. Joining constructor domains removed product cost
but invented a captured field observation. That candidate remains rejected.
The next experiment must preserve structural correlations while demonstrating
bounded relevant work; a larger unverified rewrite is not progress by itself.

The quality cadence through `7309b60b` independently found no new confirmed
finding and kept the known ignored unique integration and broader failure
gaps explicit. Root checked the typed fatal producer/shared catch boundary
against the source before accepting that claim. The next ranked delivery
is the bounded reader producer and the prepared current-pin SQLite baseline;
a further broad audit is not a prerequisite for either. Reference-writes
is the next suggested small fixture cohort, pending an author's preservation
assessment. No class closure or benchmark result follows from this review.

### 2026-11-01: verify the changed consumer graph

The native benchmark checkpoint introduces dependencies and a build script,
so the fixture-only exception did not apply. An independent locked metadata
comparison found all 115 existing package feature/edge/metadata objects and
old lock records unchanged in default and native modes. The new build output
belongs to the opt-in tool. Full default engine execution would not run its
native tests; those already have separate six-test and strict-lint receipts.
The leaf-tool rule now requires this isolation evidence, complete feature
checks, dependency policy and affected global discovery/containment checks.
Unproved shared impact still requires broader verification.

The assessor also tried expanding every repeated Cargo dependency path. It
grew needlessly and was stopped without building or changing source. The
bounded metadata comparison was already sufficient for the resolved graph;
it was not a compiler unit-graph receipt. Do not seek an absolute proof by
expanding a DAG into paths. Actual workspace discovery consumers remain part
of the final assignment. No benchmark readiness follows from this decision.

### 2026-11-01: use task operations for action

I sent the component-demand peer a new assignment through a context message
after its report completed. That operation queued a note without starting a
turn, and I incorrectly treated the follow-up as active. A later live-owner
check exposed it; I resumed the task and verified execution. Other source
tracks continued, but that did not make the missing design work progress.

The lead workflow now assigns every action through the harness's task-start
or resume operation, reserving context messages for information. This removes
the need to guess whether a note will wake an idle thread. The code-agent
handoff text also now agrees with retaining an author across corrections and
related cohorts. Execution-state claims still need observed evidence; neither
queued text nor a past assignment establishes current work.

### 2026-11-01: stop an oversized mechanism; overlap bounded delivery

The pinned-reader replay artifact exceeded its source estimate while still
leaving selected captures and pre-allocation budgeting unresolved. I retired
it as a source candidate instead of asking for another local patch or method
split. The known reader defect remains open; preserving an experiment is not
shipping a mechanism. The earlier mechanism bet has no new class-wide result.
Further reader work needs a private value/correlation contract before code.

The next refusal-fixture cohort kept one author through implementation,
mutation and final verification while a fresh reviewer read the immutable
snapshot. Eighteen families landed in roughly seven minutes from brief creation,
with an exact tested/landed tree comparison and no repeated engine run. This is
one observed delivery interval. A metadata-only jj divergence briefly blocked
landing; assigning the preserved older snapshot its own change identity kept
both trees without retesting unchanged inputs. Author execution ownership and
bounded verification held for this batch.

My schema-control brief asked for mutation sensitivity per declaration. The
author identified a foreign key already implied by other active constraints.
That syntactic quota would have added tests without independent behavior.
I replaced it with coverage of semantic invariants and an explicit implication
proof; compound mutants are labelled as such. The existing test-value rule
already prefers retained stronger coverage. No new catalog-pinning test or
custom runner was warranted. The schema controls remain under implementation;
there is no benchmark readiness or comparative timing result yet.

### 2026-11-01: independent oracles and a stopped full check

The generated-row proposal initially compared two evaluator answers. I rejected
that as a sufficient oracle: equal wrong keys would pass. The revised fixed
native-state witness checks inserted payload, key freshness, persisted membership
and direct reference linkage. Actual paired wrong-key substitutions passed the
old relationship comparison and failed the witness. The scope remains four
fixture families; no general capture interpreter or class-wide vacuity claim.

The naming repair removes two copied producers in favor of one Schema provider.
Its first full run exposed two more expectations of the old incorrect names,
in an oplog test and a manual example. Production results matched the existing
specification. The failed command remains failed; bounded consumer reruns and
all missing stages complete verification without discarding the 3528 unaffected
passes. The verification workflow now states that continuation rule explicitly.

Delivery timing identifies real costs: workspace lint 57.76s, root prebuild 380s
and root execution 401.510s. The cap-discovery guard reused that build and took
0.449s; a suspected nested rebuild was disproved. Across all 3530 duration
records, the top ten account for only 10.56 percent of summed test elapsed time.
No test deletion follows from those numbers. Compiler peak memory was not
sampled, so the need for two build jobs remains unproved. The next required
native compile may trial four with observations; no productive run is restarted.

I also let an optional cost discussion delay a lead-owned disk cleanup after its
first tool call was aborted before dispatch. The worker correctly stayed blocked.
The native cleanup later succeeded, preserving active workspaces. The lead
workflow now keeps such actions pending until an actual result and gives them
priority over unrelated investigation. This is a process obligation, not an
automatic guarantee from the tool. Keeping authors responsible for execution
held; keeping the lead focused on the critical path needs this correction.

The required native build subsequently used four jobs and passed 23 tests plus
strict lint, but the observer captured no matching compiler samples despite
reported compilation. That does not establish peak memory, a cache hit or a
speedup; the broader concurrency question remains open. The next root compile
needs a useful live sample before changing the general policy.

### 2026-11-01: keep delegation paths explicit

The six-family fixture author owned implementation, three real mutants and final
package verification; fresh review overlapped sensitivity. The complete package
reused its build and executed in 1.551s. Landing changed only read assignment
prose, so the passing package was retained. Four distinct native boundaries stay
open rather than entering a larger collection of unrelated hooks.

An approval review rejected a named local report write because no-main-edits
was too broad beside the requested main-checkout deliverable paths. I clarified
the exact output authorization immediately and kept artifact ownership with the
author. The common worker instructions now distinguish source/build ownership
from explicitly named local deliverables. Their stale cargo-clean instruction
also contradicted native idle cleanup and evidence preservation; it now follows
the existing resource gate. These corrections remove conflicting instructions
rather than asking each author to infer which one wins.

### 2026-11-01: cadence through native comparison and fixture boundaries

Fresh source-only review through `8ef7e285391b` found no new confirmed defect.
I checked shared naming consumers and the generated native witness against its
mechanism claims. No class is retired. The next consumer checkpoint remains
external acknowledged-prefix validation, with structural pinned-reader design
and a finite comparison protocol behind it. The remaining fixture boundaries
need small typed mechanisms and edit-feedback evidence, not a quota.

The crash-control author's required compile sampled four compiler jobs with
available headroom, and its eight live cuts fit under two seconds per parent
test. This observes one build's resource fit and test cost; it does not establish
a two-versus-four-job speedup or justify changing every concurrent builder.
A host process deadline is independent of the engine runtime: a narrow test-only
clock exception is preferable to starting engine I/O workers just for a clock.
Parent filesystem/process syscall liveness remains an explicit limit; this
checkpoint must not be described as universal hard wall-clock containment.

The crash checkpoint landed exactly its checked revision after source review,
mutants and scoped consumer proof; final package execution was 14.725s. One
reviewed host-clock expectation makes house-check exit 1. An initial author
report had mistaken a trailing print's exit for that command's exit; the author
corrected it before landing and the final receipt preserves native 1 explicitly.
Accepted exceptions must be recorded separately from passed checks, using actual
command exits. No source workaround or unchanged package repetition was needed.

### 2026-11-01: finish a cohort and use the host boundary

All 78 reference-write families now run from data. The final four preserve typed
load and catalog identity plus the deliberately direct restricted-old kernel
path. I rejected the unexplained 1024 catalog allocation pin; the replacement
requires actual catalog-ID versus local-offset separation and independent names.
The real offset substitution fails. Fresh review and final package input equality
justify reuse after ticket-only landing inheritance; no engine repeat was added.

The next command boundary belongs to the native process/resource manager. The
author compared group timeouts, address-space limits and custom Rust supervision;
a small systemd launcher can contain real resident/C/page memory and descendants
without copying process-management machinery. Budgets are caller supplied,
native result/InvocationID evidence is required, and unavailable killed CLI
prefixes remain invalid. Source and facility proof are still required before
claiming this boundary delivered. Successful manager units disappear quickly;
a native stop-post receipt is the chosen evidence hook, not polling or IPC code.

The native boundary is now delivered. Review found fixture reliance on an ignored
scratch parent and missing negative receipt/resource cases; one focused correction
made those boundaries executable from a fresh layout. A weaker KillMode probe then
exposed a separate cleanup ownership error: tracked PIDs missed a foreground child.
The test now uses the native owned cgroup and bounded empty/reap truth. An actual
unrecorded child was promptly killed; its earlier natural expiry is not cleanup
success. The launcher itself remained unchanged through those fixture corrections.

Executed-input provenance remains explicit: copied runtime source was recorded
before launch, full filesystem readback afterward. Immutable/source/BIN/host
closure and the affected cleanup witness justify retained component evidence;
there is no invented prelaunch full snapshot or unchanged-suite repetition.
Operational containment closes within its supported-host contract. Do not turn
an impossible universal syscall-liveness promise into another readiness gate or
add a second supervisor. Next work is the remaining reporting/finite comparison
protocol, while the completed 78-family cohort stays complete.


### 2026-11-01: reporting delivery and a contract before implementation

Reporting used one final combined native-package check after source review,
then four real contained diagnostics with independent raw recomputation. Native
execution took 14.856 seconds and the production binary build 8.058 seconds.
Those observations do not measure total delivery overhead or establish a general
speedup. The integrated file hashes stayed equal, so no engine repeat followed
landing. The approved host-clock expectation remains an explicit house exception.

The reader contract's first model preserved correlation past a native For join.
Its concrete guard oracle was too precise to reveal that mismatch. Independent
implementability review worked the actual abstract join and caught it before
Rust implementation. I ruled that scope closing follows native information loss;
the correction removes templates and tests both local closing and live outer
capture. One focused delta review approved the resulting algorithmic contract.
The native reader repair is still unimplemented and no defect class is retired.

Author execution ownership held. Source, machine and publication reviews had
separate scopes and reused their evidence. I still had to request prompt result
publication while authors prepared receipts. The existing instruction to publish
stable source and machine evidence before polishing prose remains necessary;
completion of a command must unblock the next action immediately. No further
review pass or new orchestration framework was added to address that delay.


### 2026-11-01: settle a feasible comparison without dropping a dimension

The 30-second whole-command ceiling stopped both unchanged counter peers.
Explicit pre-freeze calibration at 120 seconds completed them. The larger serial
lifecycle still failed that bound. I rejected a one-size freeze because it would
remove the requested size dimension. A bounded runtime admission rule, derived
from campaign partitions and complete counter passes, allowed an intermediate
size without changing generators, counts or guarantees. All 16 common 1000/5000,
one/four-writer cells then completed. The unused fallback was not run.

One fresh source review and one affected verification accepted the gate. Six old
successful observations were retained only after exact downstream/configuration
parity, with old binary identity visible; the future baseline requires one final
binary. The selected caller spans sum to about 9 minutes 48 seconds, while the
final native package executed in 3.376 seconds. These are scoped observations,
not a measurement of total delivery time or a general suite speedup.

The protocol's synthetic schedule checks preserve all planned rows after a cutoff
or failure. I found that an ignored mkdir failure could overwrite an existing
session index; successful exclusive creation now gates every write. A focused
mutation reproduced the overwrite and failed the preservation assertion. No new
supervisor or native containment project was needed.

Detailed Rust mutation diagnostics were lost when suite's fixed output file was
overwritten. Named failures and exits remain, so the receipt makes only that
claim. The worker instructions now require preserving native output artifacts,
not only stdout, before the next invocation. This is a process enforcer; there
is no claim that the evidence-loss class is automatically prevented or retired.

### 2026-11-01: separate measurement administration from application execution

A successful application exit did not make its missing native receipt valid.
The stop-post collector had only 200 ms; the repaired native boundary gives
receipt administration its own allowance while SIGKILL prevents the application
borrowing that time after its deadline. Real delayed-receipt and termination
mutants checked both sides. The incomplete experiment stayed inconclusive and
a fresh complete matrix used the repaired controller. No unchanged engine
build, full check or pilot was repeated for that isolated control repair.

Historical pilot reuse initially trusted a caller-selected list of self hashes.
Review caught the missing relationship before launch. The correction derives
required membership and expected identities from anchored accepted originals;
the caller selects copy locations only. Omission and current-byte substitutions
fail, and the old loop admits them. This enforces the specific provenance
boundary without another orchestration layer or a universal isolation claim.

### 2026-11-01: serial savings do not authorize a concurrent regression

The complete combined confirmation passed every correctness/provenance gate and
improved all five primary pairs, with median saving 13.754 percent. The concurrent
small counter slowed by a median 5.498 percent and failed the supported p99 gate.
Independent reconstruction agreed. I rejected the candidate rather than treating
the primary improvement as permission to change the frozen regression rule.
The acceptance boundary held; it has not delivered the requested optimization.

The author retained execution and diagnosis ownership throughout collection and
now investigates the concurrent regression from existing data and actual shared
call paths. Lower user CPU alongside higher system CPU in slow pairs motivates
separating scheduling and I/O explanations before a new source change. Repeating
the same matrix would add cost without identifying a mechanism. No broader
failure class is retired and no uncertainty is renamed a proved cause.

### 2026-11-01: reject cheap, certify survivors

The parked-wait candidate passed targeted correctness and seven meaningful
mutants but failed both concurrent pilot pairs. The rejection boundary held,
and the failed pilot avoided a full correctness check and final confirmation.
However, the pilot ran the primary workload first and deferred its decision
until all eight observations completed. Mutation execution was short while
compilation dominated its roughly four-minute cost. These stages provided
useful evidence in an unnecessarily expensive order.

The maintainer identified the iteration cost. Future candidates now screen the
known concurrent regression first, stop on that stage's rejection, and test the
primary improvement only for survivors. Expensive mutation and final checks
follow the cheap screens. Final acceptance, exact workload/oracles, native
bounds and independent audit remain unchanged. Closed rejected studies cannot
be reopened to obtain favorable points. Reuse the same author and warm build
cache after preserving immutable source, separate executables and evidence.
The next iteration must report stage costs; faster rejection is a policy until
its measured feedback time demonstrates the saving.

I also resumed an owner before its follow-up brief existed. The owner had to
wait for lead bookkeeping. The briefing rule now requires a successful file
write before dispatch, and concise evidence publication takes priority over
prose expansion. More review packets and cold workspaces do not by themselves
strengthen a boundary. Reuse unchanged evidence and review only changed inputs.

### 2026-11-01: keep development gates within the delivery contract

The first staged trial rejected after four observations in 1m48s and left its
primary runs, mutations and full check unstarted. The stop mechanism worked.
I had required both concurrent pairs to improve although the frozen final rule
allows up to 5% median slowdown. That extra requirement can discard candidates
worth confirming and push implementation toward unnecessary synchronization
changes. Future previews now use the original 5% regression line and a paired
preview of the original tail rule. Closed studies retain their original result;
there is no same-body rerun or change to final acceptance.

The normal build took 79 seconds but measurement started 8m57s after it ended.
That interval includes build-proof preparation and asynchronous observation,
input binding, resource samples and lead launch handoffs; their individual
shares are not all measured. Test execution did not explain that gap. The next
implementation brief preauthorizes one guarded development trial. Its owner
can transition from a passing normal build to the frozen screen without a new
lead notice, while the lead retains scope, promotion and landing decisions.
Reuse the established collector as an execution service, not another approval
boundary. Automate mechanical evidence with the build and avoid serial prose
handoffs. Independent raw audit remains mandatory for final acceptance, not
every rejected development preview. Measure this change before calling it fast.

### 2026-11-01: close the optimization search at its failure exit

The final composition failed its unchanged concurrent preview: 7.850% median
slowdown against the 5% limit. Four observations took about 106 seconds; primary
runs, full verification and final80 stayed unstarted. Build to measurement took
about four minutes, versus nearly nine in the earlier recorded trial. The
preauthorized transition reduced that interval in this instance, while evidence
binding still costs more elapsed time than the normal build. Focused execution
was under two seconds. Do not blame this iteration's delay on the Rust suite.

The maintainer had to ask for a failure exit after prolonged optimization work.
A measurable success threshold does not bound a search that finds no winner.
The final brief and tracked ticket now stop on rejection or the authorized total
cap; apply both success and failure exits when scoping future research. This
attempt stops after its one allowed candidate, with time remaining. Baseline and
profile are useful completed work, but no optimization met all gates. Preserve
that distinction instead of extending the search or calling the goal complete.

### 2026-11-01: test failures do not decide the architecture

The maintainer asked whether preserving old tests was trapping the work in its
current design. Some decisions challenged tests: the unsafe upsert success
expectation must change, public regression fixtures retain their intentional
baseline failures, and weak aggregate phase assertions were strengthened.
But I treated the new allocation source scanner as a completion gate before
judging its mechanism. Two valid Storage receiver spellings triggered false
positives, and the first response extended a name whitelist. That was an
implementation detail acquiring authority through a test.

The existing instruction "never delete a failing test" reinforced this mistake.
It is now replaced in code-agent.md. AGENTS.md requires assessing the governing
contract and whether code, expectation or harness is wrong. The lead workflow
requires a ruling when failures constrain behavior or architecture, and
verification.md records material dispositions in the existing receipt. Removing
an invalid assertion preserves its legitimate property through named coverage,
or explains why no contractual property existed. No extra routine approval or
failure-reporting process is introduced.

The earlier bet that enforcing mechanisms beat instance fixes still applies:
the scanner needs a justified budget-gate mechanism, not more receiver-name
exceptions. Its proposed replacement remains unverified while the goal is
paused. Existing authority compatibility was a deliberate migration scope,
not evidence that every analyzer internal is a permanent requirement. Future
reviews must distinguish those choices before asking implementation to conform.

### 2026-11-01: pin the review tree independently of author amendments

An immutable commit named in a review brief did not pin its jj workspace:
an empty child still rebased when the author amended the parent change.
The next checkout failed as stale before dispatch. Updating that completed,
empty workspace recovered access, but another ordinary child would repeat
the same condition. The review now uses a distinct duplicated change on the
original stable parent, with an observed empty full-tree comparison against
the author's frozen revision. The author can continue without moving that tree.

The lead workflow now requires this preparation for concurrent review of a
rewriting author. Keep the original revision as evidence provenance and the
copy solely as a stable review checkout. Dispatch only after checkout and
comparison succeed; a hash or attempted command is insufficient. This uses
native jj operations and adds no watcher or review approval stage.

### 2026-11-03: periodic review after fault-matrix and eight tracks

What happened: fault-matrix closed, then eight tracks ran one at a time (the maintainer's limit). Landed: every Prim named in translation, with a refusal for a pinned `print` or `parse` across a rename; a pinned composite key translated by field; create's leftover marker removed on the next whole open; typed causes in catalog, exec, write, plan and Corrupt, closing typed-causes-engine-layers. Ruled without code: `reserve_page` dropped (2.1% of suite cycles, against weakening the lost-write check); host-rule-gaps lowered to 610. Every landed tree was the tested tree, and every full check passed.

Going well: every review found something real, twice an older wrong answer beside the change that no fixed input would have reached. The one delta round rule kept each track to two rounds. Rulings that saved a track cost less than running it.

Going worse:

- Every review found a test gap a mutant survived, because briefs asked for one mutant per change. The worst was create's deletion guard, untested, where a survivor could delete what the engine does not own. Five of eight tracks needed a delta round for it. Moved to a rule: code-agent.md now asks for a mutant per guard, branch and arm the change adds.
- Four of eight tracks passed their stop line and I accepted each, so the line meant nothing. Moved to a rule: a stop line is a check-in, the author stops and reports.
- Three wrong answers in translate this session were one class, identity by name or position, fixed site by site as reviews found them. Against the mechanisms bet: the class keeps producing bugs in translate, so it has no mechanism there yet. Moved to a rule in SKILL.md: a third older wrong answer in one subsystem brings an audit of it for the class. Next: a hack-audit track over translate/.
- Typed causes took five tracks and four delta rounds while no roadmap item moved (dump-restore-backup, prealpha-app). Worth doing, but it changes no answer a user sees. Next: after picked-seed-floors, alternate a correctness track with a roadmap track, dump-restore-backup first.
- Each track resolves one ticket and opens a follow-ups ticket, mostly cosmetic. Moved to a rule in SKILL.md: such follow-ups take no priority over wrong answers, and a stale follow-ups ticket closes as wontfix.
- My slips: one outgoing-diff scan and push in a single command (scan clean, nothing leaked); a stale main working copy from a race with a slot's operation, recovered after copying the edited files aside.

### 2026-11-04: periodic review after ten tracks

What happened: ten tracks, two at a time until the maintainer went back to one. Landed: a pinned key and a written relation translated by identity; logical dump and restore (S5.1); a revert translated by the operators it applies; adtb-sim's tests built as one binary; the engine script runner with four batches (oplog, bundles, roles and server, transactions and principals), about 400 Rust tests moved onto scripts; background vacuum V1 to V4, closing background-vacuum. Every full check passed on the tree it tested, and main was re-checked when code had moved under it.

Going well: every review found a blocker, and most were false evidence rather than code: a script that could not tell a refusal at submit from a failure at completion, a longest-step test blind to work after the last step, a snapshot search that passed with the worker disabled. The script batches cut adtb-sim's Rust by thousands of lines with assertions the same or stronger, and each batch's review caught the moved tests that had quietly weakened. Rulings that tickets absorbed (twelve filed) kept each track to its scope.

Going worse:

- False evidence was the common blocker: four tests of a mechanism running beside the code (a pacer, a shared budget, a worker, a role session) passed with that mechanism off or unmeasured. Moved to a rule in AGENTS.md: such a test asserts the mechanism ran where the property needs it and is shown failing with it off.
- Stop lines were passed three more times on the vacuum track, twice after the rule that a stop line is a check-in, and once in a brief that said so in words. The authors did stop afterwards. The overruns came from writing a whole test module before running it. Next: briefs ask the author to count `jj diff --stat` before starting each new file.
- Turning on the background worker broke fourteen tests whose oracles assumed nothing else writes (pinned simulated times, page bytes, a scrub's budget). They now open with the worker off and a reason each. That switch is a debt: every test using it is a place the worker's interaction goes untested.
- The disk guard stopped three checks; each slot's build is 18 to 20 GB. Clearing idle builds fixed it each time, at the cost of a cold build on reuse.
- My slips: I misread "let the current one finish" and asked; a push to GitHub failed on an SSH key and went through later.
