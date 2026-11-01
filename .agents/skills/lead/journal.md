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
- Accepting an agent's premise, or my own, without a run. My "65 `or`s stay refused" was false.

## Entries

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
