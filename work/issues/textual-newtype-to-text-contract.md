---
created-at: 2026-10-28
status: resolved
closed: 2026-10-28
kind: bug
priority: 799
tags: [lang, quality, test]
summary: shared textual to_text strips non-Text newtype wrappers despite its print fallback contract
---

# Textual newtype to text contract

## Report

The public shared textual::to_text API strips a non-Text newtype's wrapper,
although its doc comment and docs/design/kernel.md section9 require other values
to use their printed form. The query checker only emits ToText for numbers,
times, Uuid and Text newtypes, so this is a shared library/consumer contract bug,
not evidence that query text(Cents(7)) is admitted. Do not widen the checker.

Concrete direct API witness using adtb_reference::Value as Textual:

```rust
let form = Form::Newtype {
    name: "Cents".into(),
    base: Box::new(Form::Base(Base::I64)),
};
let value = Value::Int(7);
print(&value, &form)    // Ok("Cents(7)")
to_text(&value, &form)  // actual Ok("7"), expected Ok("Cents(7)")
```

Observed in an external target-only Rust probe on frozen UUID state
0cea33e5ae69645c266a5e5c049263ff9473dcdb, base36499281. The generic Newtype arm
is unchanged from main; do not attribute this to the new UUID branch. Probe
source /home/tony/code/adtb-uuid-format/target/uuid-format/generic-to-text.rs;
compile+run logs and exact commands generic-to-text-{compile,probe}.{json,log}.
Compile0,0.642s; run0,0.024s. No tracked source edit was made for the probe.

Root cause: crates/adtb-lang/src/textual/print.rs to_text matches Newtype, preserves
only a Text special case, then recursively calls to_text on its base. That turns
a valid Cents value into the base literal. UUID fix deliberately invokes print
for its fallback and does not touch this preexisting branch.

Next: fresh focused track after UUID landing, fix shared contract for valid
non-Text newtypes, preserve Text newtypes' bare text and base/UUID behavior.
Use existing scalar/print helpers, no per-value parser/allocated Form or query
API expansion. Tests must demonstrate print fallback for I64 and Bytes newtypes,
Text special case and canonical16/non16UUID controls. Narrow lang + relevant
consumer tests, strict changed-crate clippy/fmt/house/size, independent review and
lead full check. No new design ruling or spec change is currently indicated;
report an actual contradictory contract before changing one. Provider features
remain behind general engine correctness; native bootstrap remains its own repair.

## 2026-10-28: bounded shared contract repair dispatched

Fresh successor textual-to-text after reviewed UUID source lands; base includes
that code and current rulings. Own /home/tony/code/adtb-textual-to-text, one writer,
local brief work/briefs/brief-textual-to-text.md. Read/edit while lead runs frozen
UUID integration; no cargo compilation until lead releases slot. Landing order
UUID first, then this shared textual library fix after UUID full check passes.
Existing lang textual test explicitly expects bare Cents; revise its contradictory
expectation rather than deleting coverage, preserve Text/UUID/base controls and
add Bytes-newtype fallback. Existing kernel9/doc are the consumer contract; no
query builtin expansion, dependencies or spec change. Capture failure on old
implementation using tests first, then common fix. Assigned branch scope affected
lang/textual/consumer tests, strict lang clippy/fmt/house/size; lead owns fresh
review and full post-land root/app/policy/docs check. Keep exact frozen IDs,
source/build/config inputs and honest status/timing receipts.

## 2026-10-28: resolved

Landed 2a44d5ab0facfa4c18ce128aefa1efee054e2831. The shared generic Newtype arm
uses print(v, form) for non-Text newtypes; the Text special case is unchanged.
No query builtin, contract, spec, ruling, dependency or signature changed. Existing
Cents coverage was corrected and retained; schema-derived Bytes controls include
empty/high bytes, roundtrip and wrong scalar kinds. The original baseline on
b2445dc7854b5fb5178ff68516acfa26550a99a8 collected 100: ten passed, two failed.
Frozen e0659096d9ff3948a759c746a650ee1b84c87e3f passed language textual tests (12),
reference textual tests (16), engine textual tests (3), strict language clippy,
format, house and size. The new test took 0.009 seconds. All tracked/config inputs
remained unchanged during each check. Fresh independent review found no findings
and rehashed all 2,560 files, 14 config paths and 12 evidence pairs.

The full check passed on clean main working copy
51a19354230b830a89023a81ae61cc439347e51f, parent 2a44d5ab.
Command from main, retained in target/textual-to-text-integration.sh:

```
direnv exec /home/tony/code/adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target check
```

Rust 1.98.1 / Cargo 1.98.0; default configured features and profiles. Collected
exit 0. Start 2026-10-28T18:13:43Z, end 2026-10-28T18:16:06Z: 143 seconds, with one-second
timestamp precision. Permission, queue and execution-only times are unknown.
All 2,396 root tests passed, eight skipped (50.521 seconds). All 47 campaign
tests passed, one skipped (20.744 seconds). Clippy, format, parity, dependency,
size and docs checks passed. Docs: ten tests in 10.238 seconds, 420 syntax inputs,
61 pages, 6,734 links, 4,999 fragments, no failures.

All tracked inputs, local root/app Cargo configs and revision remained unchanged.
Evidence under target/textual-to-text-integration/; check.log SHA256:
1e618e72f228e0b650da12d1387792e9999dcb25c09ad9ef70fca7a793c1550f.

No unfinished integration scope remains. Historical baseline failures and source
remain preserved. Subsequent ticket-only updates use tix/house validation; they
change no executable or book input. Pushing is deferred while Tangled is down.
