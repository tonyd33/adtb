---
created-at: 2026-10-28
status: open
kind: bug
priority: 799
summary: bundles without a module import resolve the default Principal instead of the custom root type
tags:
  - engine
  - lang
  - quality
  - test
---

# Custom principal global lookup

## Report

During the shared principal validation baseline, sourceafd90d40 on c14a6594
opened a custom Principal in module identity and registered a bundle with:

```adt
query me () -> Principal = caller
```

Both fixtures failed open with RegistrationRefused/Invalid:

```text
query `me`: expected Principal, found Principal
```

Adding import identity let registration proceed and exposed the independent
asserted-value bug. That import is NOT required by security1.1: the schema's
Principal belongs to root and resolves from every module and bundle. Adopted
spec/schema-principal/root-first is the authority. The apparent mismatch is
consistent with unimported signature lookup selecting the fixed prelude type
while caller has the custom type; exact resolver site still needs audit.

Evidence: adtb-principal-validation/target/principal-baseline-before-import,
originalafd90d40 and captured RegistrationRefused in both tests. The initial
setup-error diagnosis was wrong about user validity. This is not a successful
principal-validator baseline because it fails earlier, but it is public engine
lookup evidence. Reproduce a minimal exact-current main regression, fix the shared
lookup boundary across modules/bundles and test default/custom/import controls.
No import workaround, changed adopted root semantics or new design choice.
Current asserted-principal-value-domain remains separate and does not claim this
resolved; finish its frozen checks before another source change. All provider
and campaign expansion remains paused behind engine correctness.
