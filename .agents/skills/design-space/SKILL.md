---
name: design-space
description: Explore a large design space with the user as questions and options kept in a quandary directory, expanding nodes, raising potential defeaters and prior art, and recording the user's rulings in points. Use when the user wants to explore, prune or decide among design alternatives (language features, syntax drafts, mechanisms), says "design space", "what are the options", "defeat X", "expand Y", "rank these", "backtrack to", or works in a directory of quandary question files. Not for judging an existing implementation or for a single yes/no design call.
---

# Design space

The user and the agent search a design space together. The space is what
the agent writes: questions, each with options; an option, when explored,
raises further questions; an objection to an option is a question raised
by it, whose options are the ways to resolve it. Rulings are where the
user stands among the designs the space admits, kept in points. The user
owns every judgement: preferences, acceptances, defeats, and whether a
potential defeater defeats. The agent expands nodes, gathers evidence,
keeps the record, and runs the tool.

## The tool

`quandary` reads a space directory: `quandary.yaml`, `questions/`, `notes/`,
`points/`; `quandary init` creates them, and a directory without
`quandary.yaml` is refused. Its
README is the format reference; read all of it, not a slice, before
writing any file: `README.md` in a quandary checkout, or
`https://raw.githubusercontent.com/tonyd33/quandary/main/README.md`.
Run it as `quandary` where it is on PATH, or
`nix run github:tonyd33/quandary -- ...`. `cd` into the space; never store the command or its flags
(`-C`, `--point`) in a shell variable, which zsh does not split. Use a
shell function if anything:

```
quandary init           # create a space here, or fill in what it lacks
quandary check          # after every edit; prints ok
quandary lock 'who: what'  # before an edit touching more than one file
quandary unlock         # after it; checks the space, then releases
quandary status         # the frontier, in decision order
quandary show Q         # a question's options, verdicts, rulings, what each raises
quandary show O         # one option: its question, ruling, why forced or impossible, body
quandary why O          # every set of rulings that rules O out or forces it, and the constraints it goes through
quandary whatif defeat O1 O2 accept O3   # what would move; what an accept commits to
quandary accept|defeat|exhaust|reopen|defer|prefer ...
quandary accept O1 O2 -- reason   # several at once, all or nothing
quandary exhaust Q1 Q2 -- reason  # likewise
quandary settle Q1 Q2 -- reason   # what is taken is all of it
quandary rulings        # the point's rulings, inherited included
quandary diff P1 P2     # how two points differ
quandary links          # link labels in use
quandary adopt BRANCH   # take a branch into the point it extends
quandary point new NAME [EXTENDS]
quandary point rm NAME
quandary --point NAME ... # work at another point
quandary graph          # Mermaid
quandary graph --format dot  # Graphviz, for large spaces
```

The user can read the space directly, live, with `quandary browse` in a
terminal or `quandary serve` in a browser. Suggest it when they ask to see
a question or the status again, instead of restating `show` in chat.

Rulings go through the commands, which print what moved, one line per
question. Put every reason in single quotes after `--`
(`accept a b -- 'won'\''t scale; too slow'`): unquoted, `;` ends the
command and an apostrophe opens a quote; show that output to the user as printed, not retold. The agent never edits a point's frontmatter by hand; its body is free prose, for notes such as why a branch was not taken. Structure (new
questions, options, constraints, links, notes) is written by editing the
files under `questions/` and `notes/`, then `check`. Before any edit
touching more than one file, or leaving one half-written between tool
calls, run `quandary lock 'who: what'`, naming yourself and the edit;
rulings refuse until `quandary unlock`, which checks the space first.
The lock lapses 15 minutes after the last edit. When your `unlock` says
`not locked`, the user released it or it lapsed: say so, and `check`
covers the edit. A problem `check` reports in a file you did not touch
is the user's edit in progress: leave it, and tell the user if it is
still there at `unlock`. Pass on any `ruled while locked:` lines
`unlock` prints. A warning that an edit lock lapsed without unlock means
an edit may be unfinished: tell the user before relying on that part of
the space. When a ruling refuses because the space is locked, tell
the user who holds it; add `--force` only when they say. The user may
rule while you edit: after your edit, re-run `show` on what you wrote
before describing verdicts, and when one changed, `rulings` names what
the user ruled.

## The loop

1. **Pick the node.** Start from `status`: the frontier lists open
   questions in decision order, options by the user's preference. Work on
   what the user names; otherwise propose the top of the frontier.
   Every time the work moves to a node, re-present it before anything
   else: what it is with code, where it sits (its question, rivals,
   raiser), and its open questions with their options. The user does
   not hold the space in their head.
2. **Expand.** For an option, write each question exploring it raises as
   a question file with `raised-by` the option, named for its topic in a
   topic directory, not for the option that raised it. For a question,
   list its options, including ones the user will dislike: the space is
   searched, not argued. `pick: one` when exactly one is chosen, `some`
   when at least one of several is, `any` when any number can coexist. Every question and option gets a
   description that a later session can act on without this
   conversation: the question's in prose before its first `##` heading,
   the option's in its `## <option id>` section, saying what it is, with
   code.
3. **Evaluate.** For each option:
   - Precedent: which systems did this, with the primary source (paper,
     repo file and line, spec section). Unverified claims are marked so.
   - Potential defeaters: each is a question raised by the option, its
     description the objection, its options the resolutions (what would
     have to change to overcome it: a spec rule, new machinery, a decided
     ticket reversed). Always list living with the cost as a resolution,
     so the user can dismiss the objection by accepting it; it
     `excludes` every other resolution. Resolutions usually combine:
     `pick: any` unless they truly exclude each other.
     Give each objection its own directory, its file named after it, so
     its option keys are its own:
     `questions/feed/slot-failover/slot-failover.md` is question
     `feed/slot-failover`, with options `feed/slot-failover/live`,
     `feed/slot-failover/upgrade-17`. An
     objection that holds for any of several options lists each:
     `raised-by: [a, b]`. Options of a `pick: one` question must be true
     alternatives; layers that stack go in separate questions. A raised
     question that is a plain choice, not an objection, needs no "live
     with it" option.
     An objection that holds only for two options together is raised by
     both: `raised-by: [[a, b]]`. Accepting a resolution commits to a
     raiser; to say "if a, then this fix", defeat the other fixes and
     exhaust the question instead. "Unsound by construction" is a
     defeater with no resolution.
   - Constraints: `needs`, `needs-one-of` and `excludes` when they are
     hard, `forbid` for anything else hard; everything soft as a
     labelled link (`rivals`, `gains`, `variant-of`, `same-as`,
     `resolved-by`, `needs-capability` to a note). Reuse a label `quandary
     links` lists before inventing one.
   - Search order: evaluate options in the user's rank order, and test
     first the objection most likely to kill an option. When grading
     on an ordered scale (say broken, hack, costly, clean), an option is
     worth no more than its worst objection and an objection no more than
     its best resolution:
     stop grading an option at its first objection with no resolution
     better than hack, and stop an objection at its first clean
     resolution. Once one option reaches clean, test each sibling only
     for "can this also reach clean?". The cutoffs prune on soundness
     only; the user's preference decides among options that survive.
     Record a grade as the option's `assessment` once the space lists
     its grades in `quandary.yaml`, each with a `desc` saying what it
     means; `show` then gives each option's bounds and which siblings
     it dominates. An option's bounds span every option its designs
     commit to, including questions raised by the options it needs; a
     graded listed option is enough to get past a question's `.other`.
     `quandary next` names the fewest options whose grades raise or
     lower the most preferred root option's bounds; grade those next.
   Analysis, tables and code go in the bodies.
4. **Present.** Show the user the new nodes and defeaters with code, not
   coined labels: before and after snippets, the concrete desugaring, the
   example query. Ask for their preferences and rulings.
5. **Record the rulings** with the commands: `prefer`, `accept` with the
   reason, `defeat` with the reason (an upheld defeater is the option
   defeated with the objection as reason; a dismissed one is a resolution
   accepted), `exhaust` when the listed options are all there is,
   `settle` when what is taken is all of it, `defer`
   for set-aside questions. To set aside one option, move it into its own
   question and defer that. Reasons are the user's words; when the user
   gives none for a defeat, ask; never write a placeholder. An accept
   without a reason takes none; never repeat the option as its reason. Skip a ruling on an
   option the tool already reports forced or impossible unless the user
   wants it kept. Facts the user states about the environment ("Redis is
   managed, no modules") become one root `pick: one` question per fact,
   its only listed option the fact (`.other` stands for the rest),
   unless an existing question already answers it, which
   the user accepts, with `needs` and `excludes` doing the pruning; never
   defeat options yourself on their strength. A fact you assume rather
   than heard is accepted only in a proposed point. Show the printed output. A
   ruling that would leave the point stale is refused; show `why`. When a
   ruling leaves another point stale, stop the batch and tell the user.
   Run `whatif accept` before any accept and show the user its output: it
   shows the raisers the accept commits to, and under `raises` every
   question the option raises with its options' bounds at the point the
   accept reaches, so the user sees what the option commits them to
   where they choose it. Before reopening a ruling to unblock
   another, show the user the conflict `whatif` names and ask.
6. **Branch.** To work out a question whose raiser is undecided, make a
   point extending `current` that accepts the raiser (`quandary point new
   NAME current`, then rule with `--point NAME`). A branch may rule against
   what it inherits: its own ruling replaces the inherited one, and
   `reopen` of an inherited ruling drops it there. Compare branches with
   `diff`, and take one with `adopt`. An example is a point that accepts
   what the example assumes; it is stale when the space no longer admits
   it, and it should settle every open question the example's code
   depends on. A branch or example you work out beyond the user's
   rulings is a proposed point (`point new NAME current --proposed`): rule
   freely there, giving each ruling your argument as its reason, present
   its rulings, and `adopt --confirmed` only after the user confirms or
   rewords them. When the user takes it with changes, make them in the
   proposed point with the user's words as reasons (`--point NAME`
   accept, defeat, reopen), then adopt it; never retype it into
   `current`. A reason the user did not give is reworded or cleared
   before adopting. Never rule in `current` or a point that is not
   proposed on your own.
7. **Integrate.** After an acceptance, read `status`, its stale points
   first, and look at the examples for awkwardness no constraint records.
   Turn what you find into links, defeaters or new questions.

## Rules

- Never rule on the user's behalf. Never defeat an option yourself unless
  it is blatantly unsound; raise potential defeaters and let the user
  judge.
- Never delete a defeated option or a question. Backtracking is `reopen`
  and following `raised-by` back up.
- Never invent a preference; an unranked option stays unranked.
- Research before recommending: read the code and primary sources. A
  recommendation is allowed and useful, but it is marked as yours.
- Do not rename nodes without the user; `check` reports dangling
  references after a rename.
- Keep bodies terse, with no counts that go stale; name the command that
  produces them instead.
