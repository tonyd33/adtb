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
