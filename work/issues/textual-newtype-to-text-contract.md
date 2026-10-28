---
created-at: 2026-10-28
status: open
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
