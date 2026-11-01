# Private pinned Value correlation contract

Status: accepted internal contract for a candidate field-observation domain.
Public authority entry points retain their existing exact evaluator, policy,
budget and Requirements. Activation requires native parity and allocation checks.

## Recommendation and exact reference

Use immutable, component-factorized Values whose facts carry reduced ordered
finite-domain decision diagrams (MDDs). Do not use a list of whole guarded Values.
The reference is the current native analyzer, expanded by its existing For
alternatives, including native joins, transport suppression, Keyed suppression,
partial evaluation order and upsert old/proposed behavior. It is not a concrete
SQL/data evaluator, scalar SAT solver, or improved authority analysis.

For every assignment of live For-choice axes, decoding a symbolic Value must
produce the same native abstract Value and emitted observations as that expanded
execution. Native joins within that assignment still union reads/stored origins,
clear unequal copy/key witnesses, join bindings and changes, and retain the
native may_empty rules. Guards only encode alternatives the native For expansion
already separates. A Case does not invent a new correlated decision for an
unknown parameter or a scalar If. This restriction prevents an accidental gain
in precision from suppressing native reads on repeated parameter matches.

The field result is the union of exactly those emitted ReadField identities
whose guards are satisfiable. Exact empty is a valid result. Exhausted budget,
invalid kernel, allocation failure and incomplete handling are errors, never
empty reads. No fallback drops facts. No Reader activation is justified by the
Python model alone; full native field/authority parity and the owner controls
are required before activation.

## Typed identity and binder scope

Use distinct private newtypes AnalysisId, ValueId, GuardId, AxisId,
InvocationId, CopyFamilyId and SnapshotId. Catalog FieldId, ConstructorId and RelId
remain their existing identity domains. A field spelling or path locates a
component within a checked Value; it never identifies a stored origin or draw.
No ID crosses an analysis invocation. Arithmetic exhaustion returns TooLarge.

An AxisId denotes one native For binding's choice of a source alternative,
instantiated in one symbolic invocation frame. Its finite domain is exactly the
native source alternatives, not all constructors of its declared type. The
axis is fresh even for two binders evaluating the same Rel term, same relation,
same constructor and same snapshot. A source term or catalog RelId is not a draw.

Let, Var, Field and aliasing a captured row retain its ValueId and free axes.
A retained partial Let evaluates its initializer once in the original frame;
refining a later Case never reruns that initializer. Copying a handle does not
freshen its axes, copy witnesses or snapshot. Case payload binding restricts the
already evaluated scrutinee; it does not rerun the scrutinee.

A Set holds the native joined element shape, guarded only by still-live outer
axes. No bound producer templates survive a For return. Ranging an alias of a
set creates a fresh draw from that joined source's current alternatives. It does
not resurrect producer histories that the source's native join already erased.
Singleton creates no source alternatives beyond its element's actual shape.

Create an InvocationId for each entry to native eval and each native loop/body
continuation invocation, including repeated evaluation of one term in distinct
Fix iterations. The invocation record contains its parent InvocationId and the
set of currently active symbolic draw axes. A symbolic execution stands for the
family of native invocations indexed by assignments to those axes; a witness is
(generating InvocationId, local fresh-event ordinal, captured axis assignment).
The event ordinal distinguishes multiple fresh() calls within that invocation.
Different symbolic eval entries have different InvocationIds even for one Tm.
Repeated Fix iterations in a native joint operation receive new invocation IDs.
Do not memoize effectful evaluation by term and environment alone.

Copy and snapshot witnesses have separately typed family handles. Equality first
compares invocation and event identity, then assignment coordinates captured at
generation. Captured aliases keep that witness without allocating a new family.
A newly evaluated relation/At allocates a new family even if its term is reused.
Generation before a local draw excludes that draw from the witness; generation
inside its body includes it. Componentwise equality can compare witness families
and coordinate guards without enumerating unrelated axes. Families with different
invocation/event identities are unequal. This is semantic token equality, not
alpha equivalence of bookkeeping IDs.

Keyed retains its native relation, snapshot witness and transition. At creates
its snapshot in its actual invocation frame. For records a dynamic entry marker;
unkey_after removes witnesses whose snapshot creation descends from a body
invocation after that marker, exactly as the native fresh-number floor does.
An inherited snapshot created before entry is retained. Returning from At restores
the parent's current snapshot. No inference that a repeated At point is independent
of the binder may strengthen the current native keyed suppression behavior.

## Guard algebra

Guards have False and True terminals and nodes (AxisId, child per alternative).
All paths use one deterministic AxisId order. Reduce a node with equal children
to that child and intern equal nodes. Domains have at least one alternative;
an empty source has no live element, not a zero-arity truth node.

AND and OR use the ordinary ordered-diagram apply recurrence: split on the least
axis present, recurse once per alternative, and reduce/intern the result. Memoize
by full operator/operand IDs. NOT complements terminals and recursively maps
children. Restriction substitutes a known axis alternative; constructor
restriction is AND with the scrutinee's existing constructor-presence guard.
Relative negation under path P is P AND NOT G. An arm-complement is not a scalar
predicate proof. Satisfiable means reduced GuardId != False; there is no separate
unbounded SAT search. Memoization, interning, width scans and restriction all
consume the same checked work account. Never expand into DNF or assignment lists.

For a satisfiable path P, the guard of an emitted fact f is P AND fact_guard(f).
Duplicate (component, typed fact) entries combine with OR, retaining the full
producer disjunction. Equal FieldIds do not imply equal histories. Selecting
between captured producers combines each producer's fact guard with that
selector arm's existing guard before OR. Subsequent observation adds its own
path; this is where incompatible producer histories become False.

## Value algebra and native joins

A ValueId points to a checked shape DAG. Constructor and field edges have
presence guards; each component separately maps typed reads, stored origins and
native metadata to guards. Records retain independent child ValueIds rather
than distributing children into complete tuples. Read guards attached to a
parent remain separate from descendant guards. Set reachability and may_empty are explicit. Bottom and an empty reachable set differ.

Projection selects existing field edges, ANDs their presence, and ORs only the
selected children's facts plus inherited parent reads. Missing fields contribute
Bottom under that guard. Projection does not consume carry fields merely because
it transports them. Whole consumption follows every reachable component; the
multiplicity walker suppresses descendants only where the native decoded Keyed
witness exists. It cannot use a may-have-keyed fact as a must-have witness.

Join is pointwise native join on decoded worlds, not arbitrary guarded union of
metadata. Reads and stored origins use OR. Copy/key witnesses are retained where
all overlapping non-Bottom contributors have the same native witness; disagreement
is None on that overlap. Disjoint paths may retain different witnesses. Native
If and parameter Case arms can overlap, so their distinct witnesses clear there.
Presence, may_empty, write binding and Changes.replaced use their current native
join/intersection rules, including copy equality for replaced rows. A proposed
row has no stored origin unless native transport from a stored row supplies one.

Case evaluates the scrutinee once, emits its native constructor observation, and
executes each native arm under the conjunction of current path and that arm's
constructor-presence guard. Its payload aliases the restricted scrutinee. For
introduces a fresh draw axis and evaluates the body once symbolically, then closes
that draw with the native fold below. It does not distribute unrelated sibling
components into complete tuples. Native
scalar If still evaluates both branches and consumes its condition; no numeric
truth reasoning is introduced. Reject/Fail and partial Let retain native order
and reachability. In particular no failing initializer moves under a later Case.

## Closing a local draw

For entry evaluates source once. Its element alternative keys are exactly
Value::alternatives keys: one per present top row constructor, no alternatives
for Bottom, and one for other shapes. Under outer guard P the constructor k draw
has path P AND (draw=k) AND source_presence(k). Payload is the selected source
alternative; copied aliases of that payload retain its draw and native witnesses.
The For's own result starts from the native empty-set/Bottom accumulator.

Before exporting the result, fold the local draw's contributions with native
join separately for each fixed assignment of still-live outer axes. Implement
this fold componentwise. Let E_d(G) = OR over every domain alternative of G with
d substituted. E_d is a metered MDD recursion, not assignment enumeration over
outer axes. Reads, stored origins, constructor/field presence, reachable paths
and may_empty use E_d with their native join guards. Native Set union's explicit
may_empty AND, when applicable, runs before this closing fold; it is not replaced
by a blanket OR rule for every set operation.

For witness t on a component with contributor-presence guard H and witness guard
T, retention is E_d(H AND T) AND NOT E_d(H AND NOT T). This says at least one
contributor has t and every present contributor has that same t. Missing witness
counts as disagreement; Bottom does not. If t is a family indexed by d, partition
its semantic instances before the test: different captured d coordinates denote
different tokens even when their printed family handle is identical. Apply the
same rule to Keyed including snapshot/transition equality. Do not existentially
merge a copy family into one token before checking equality. Fields/constructors
are joined at their existing component paths, so absent child fields do not count
as contributors. Recursively fold binding and nested shape components.

Changes fold uses the native reachable-alternative identity: no reachable
contribution gives unreachable; made is the structural union over reachable
alternatives; replaced retains an entry only if every reachable contributor
contains the same relation and a nonempty equal native row-copy witness. Native
Changes::then stays sequential, not this alternative fold. Where structural
change equality involves joint payloads, use the joint continuation below on
that change component's support, not on the entire environment.

After closing d no exported Value/fact/witness depends on d. Live outer axes are
unchanged. There are no bound-template alpha-equivalence operations. Ordinary
structural state equality, including semantic witness equality, is authoritative.

Example: For x in A|B returns the same C constructor with p=storedTone only for A,
and tag=Tag(x). Closing x yields C.p with tone and C.tag with A|B. A later For sees
one C alternative. Its tag-B arm consumes p and emits tone. Retaining an x history
through that boundary would incorrectly suppress tone. In contrast, if x remains
an enclosing active draw while an inner singleton captures p, closing that inner
draw preserves x. A later x-B observation of an x-A stored capture emits no tone;
x-A observation does. An expired local axis and a live outer axis are distinct.

Selecting between captured a and b under live selector s retains
(s=A AND a=A) OR (s=B AND b=A) for A.tone. Observe before a,b,s scopes close,
as in a comprehension over a,b,s followed by singleton selected capture r. Native
scope exit subsequently joins those scopes in order. Unknown parameter Case arms
can overlap; they introduce no invented constructor-choice axis.

## Joint continuations and dependency closure

Count, Sum, Avg, Min and Max stay componentwise: evaluate body with the native
binder boundary, consume the same input/multiplicity components as aggregate.rs,
close local draws, and return the same native result shape. Count must not decode
all record components or enumerate every persistent-environment axis. Keyed
suppression is the existing native ancestor/component rule lifted to guards.

For replacement/change equality, user-aggregate state loops and Fix, use one
existing native continuation on decoded ordinary Values. Split operand evaluation
from that continuation where needed: evaluate semantic operands once in native
order, then pass their decoded Values. Never evaluate operands symbolically and
rerun their terms inside native dispatch. Retained branch/lambda terms receive
their required decoded free-variable environment. Fix starts its loop only in the
native continuation; its definitions are not evaluated once outside the loop.

Construct the dependency closure from the continuation's semantic operand Values
and the Values of kernel free variables in its retained terms, excluding variables
bound by the continuation itself. Walk only those Values' actually referenced
guards, shape components and semantic witness coordinates. Add dependencies
transitively from witness-family creation frames and current path. A variable's
presence in persistent Env alone is not a dependency. If a native continuation
can inspect a whole operand, all of that operand's shape components are semantic
dependencies. This finite closure is conservative and exact; do not omit an axis
merely because it currently emits no ReadField.

Stream assignments to the closure's axes in fixed order under current path,
skipping unsatisfiable prefixes by MDD restriction. Do not materialize their
Cartesian product. For each assignment use one shared decode table for every
argument/free Value and witness instance. Reuse token/snapshot identities in that
table for captured aliases, and keep independently generated instances distinct.
Decode each component by its guard truth and native shape/witness meaning. The
shared table avoids separately decoding an old row and its alias into fresh tokens.

Precondition: preserve the ordinary current native analyzer context, including
the current snapshot, at the continuation handoff; do not initialize a default
context.

Invoke the SAME native continuation once for that assignment with decoded
operands/free environment. Every owned allocation and visit, including its exact
For/Fix work, debits the shared account. Its internal For returns perform ordinary
native join; no local guard axes enter native Fix state. Compare Fix states with
the existing native structural equality and actual native copy/key witness equality.
New iteration invocations generate new native token/snapshot identities, so an
alpha-renaming cannot make distinct fresh tokens equal or prevent their native
join from clearing them. Native convergence and retained partial-evaluation order
remain unchanged.

Import the native output and emitted observations under the assignment guard by
componentwise native join into the guarded result. Native-generated witnesses
carry their new invocation/event identities; witnesses originating in decoded
arguments map back through the shared decode table. A table miss for an output
native token creates a fresh typed import identity, never a spelling-derived or
position-derived substitute for an existing token. Repeated occurrences of that
native token use the same table entry. A metered reverse map provides this.

The closure scan, streamed assignment stack, decode/reverse maps, native calls,
import recursion and result joins are part of the same precharged budget. This
can grow exponentially for genuinely joint state. It gives no polynomial bound
for Fix or user aggregates and no resource guarantee for every previously admitted
input. It introduces no new error class, fallback or budget ceiling. Native
field/authority parity and owner registration controls remain activation gates.

## Work and allocation before construction

Use one monotone checked work ledger for this field analysis, at the unchanged
MAX_STEPS ceiling 1<<20, with the existing depth ceiling 64. A debit covers each
visited native term/shape/type edge, guard apply/restrict state, compared key or
child, environment lookup step, renaming step, copied byte and container capacity
unit. Charge variable-sized comparisons by their inspected size. Failed debits
return the existing explicit TooLarge outcome before the operation executes.
The native authority analyzer keeps its current accounting and behavior.

Define requested-capacity units from the actual representable node/container
layouts and charge checked capacity * size_of(element), plus explicit owned
header/arena bookkeeping. The account covers requested capacity and work; it does
not promise allocator rounding, malloc residency or process RSS. Borrowed checked
input/schema memory is excluded. No second byte ceiling or enlarged work ceiling
is introduced. Actual layout units and a sealed fallible Rust allocation API are
implementation gates, not an inferred byte limit from Python logical cells.

All new analysis-owned allocation belongs to one budgeted arena/container layer.
Reserve the complete capacity increment BEFORE allocate/reserve/clone; account
both old and new requested storage during resizing and explicit bookkeeping.
Borrow checked input/schema strings and types where possible. If copied, charge
their complete byte lengths before allocating. An immutable handle clone is free
of heap work; any deep clone is an explicit metered traversal. Fallible allocator
failure propagates separately. No hidden allocation via derive(Clone), collect,
formatting on the success path, recursive temporary vectors or map entry growth.

Covered boundaries are: arena nodes and chunks; guard child arrays; unique and
apply caches including misses/hits/keys/resize; relation constructor/key/field
metadata; substituted type expansion and unknown shape creation; shape and fact
maps; parent-marker inheritance; environment frames/lookups; local-axis closing/restriction stacks and witness partitions; token/snapshot families; alternative iterators or materialized
lists; Changes/binding/replaced collections; aggregate/fix previous-state storage
and equality; final observation dedup/output; recursion/explicit work stacks.
Existing unmetered native helpers cannot be reused in this path before reserving
their entire upper bound or making their construction budget-aware. In particular
unknown(type), env.clone(), elements(), alternatives(), node.clone(), and consume's
read Vec currently allocate before any such account. A wrapper after them does
not satisfy this contract. Borrowed preexisting input memory is excluded explicitly.

Guard/Value/env arenas retain storage until analysis end, making high-water
accounting monotone and avoiding release-credit bugs. Persistent parent frames
replace full environment clones. Each frame insertion allocates a metered fixed
node; lookup meters every ancestor examined. Recursion depth and work stacks
are checked before push, not after allocating a deep recursive skeleton.

For n independent fixed-width component captures, retained shape/fact data are
O(n); straightforward repeated OR can retain O(n^2) guard nodes/work, as measured
by the model. Doubling n should therefore approach factor four in that family,
not width^n. This is not a general polynomial guarantee. A jointly controlled
fact can require exponentially many ordered-diagram nodes; bad variable order,
large constructor domains, repeated apply pairs, joint write/fix operations and
output shape can also grow. Every expansion is debited first and fails explicitly
at the existing ceiling. No successful empty result substitutes for exhaustion.

## Enforcers and integration gates

Types enforce separate identity domains, opaque arena ownership, immutable Value
handles and one fallible budgeted constructor interface. A searching lint/test
must reject unmetered allocation/deep-clone methods in the new path; existing
native authority code is outside that restriction until migrated separately.
There is currently no such type/lint implementation. Search tests must check
local-close versus live-outer preservation, per-assignment native parity,
constructor restriction/complement, parent-marker inheritance, keyed suppression,
copy equality, snapshot escape, partial Let order and all allocation sites.

The bounded model checks guard truth tables, active captures/aliases, selection,
local-close versus live-outer distinction, same-constructor p/tag field join and
all-present-contributor copy/key witnesses. It checks logical node precharge,
not Python allocator bytes. It is a small source-guided abstract domain, not a
kernel interpreter. Native Rust execution, general shape projection, parent-marker
inheritance, Changes, Fix, dynamic invocation instrumentation and allocation sealing
remain implementation gates. Generated tests must check they reached their cases.

Practical seams: permission/value.rs holds structural algebra; eval.rs has native
Let/Case/For/project boundaries; shape.rs creates unknown/stored values; mod.rs
owns budget/env/consume; aggregate.rs and write.rs contain joint-state operations.
A private value-domain policy can share these kernel operations while authority
uses the existing exact domain. No public Requirements or Reader signature changes
are needed. The guarded mode must not silently reroute authority into its meter.

## Strongest rival and revision conditions

An exact lazy whole-Value decision DAG with persistent environments is the
strongest simple rival. It preserves the same captures/aliases/selection without
an attribution ledger and can share common branches. However terminalizing a
record of n independent binary stored/pure components needs 2^n distinct tuples;
four-constructor enumeration has 4^n assignments. The proposed component factoring
retains independent child guards and does not build those tuples. If the rival
keeps records as independently conditional children instead, it has adopted the
essential proposed factorization and is an implementation variant, not a defeated
idea. The model computes enumeration counts; it does not run a rival Rust engine.

A complete attributed-demand rival could also be exact and compact, but under
the same captured/selected examples it must retain producer histories, distinguish
own/inherited markers and precharge all metadata/env allocations. A replay mechanism needs both obligations too; excluding them is not evidence
that its implementation is smaller than the guarded domain.

Revise the recommendation if fresh review finds a scope closing that cannot preserve native joins
without product work in the independent family, native pointwise
witness/partiality parity fails, allocation sealing requires disproportionate
new machinery, or a rival implements the SAME identity/selection/budget contract
with demonstrably smaller scope. The recommendation is for this private
representation, not a claim that native integration has been completed. 
