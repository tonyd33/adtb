---
title: Constructible UUID values fail total formatting
---

Observed on engine/reference source5debace3ea0f97765bc72282bbb0d721b8cfc851,
evidence-only fixture statea1a1e3d261e9f303d9c68245a97a384ff44448bf in isolated
adtb-uuid-audit. Command from that root:

```
direnv exec /home/tony/code/adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target cargo nextest run -p adtb-sim --test uuid_contract_evidence --success-output immediate --failure-output immediate
```

Collected exit100,4tests:2evidence tests passed,2contract regressions failed.
Command wall62.373s including cold compilation; test phase0.131s. Inputs stayed
unchanged. Local log target/uuid-evidence.log in that workspace; local fixture
crates/adtb-sim/tests/uuid_contract_evidence.rs. The tracked ticket
work/issues/uuid-domain-textual-roundtrip.md preserves rerun scope.

```
Uuid(bytes("x"))          -> Bytes([120]) at type Uuid
unwrap(Uuid(bytes("x")))  -> Bytes([120]), printed 0x78
text(Uuid(bytes("x")))    -> Internal("a `Uuid` is 16 bytes, found 1")
print(Uuid(bytes("x")))   -> Internal("a `Uuid` is 16 bytes, found 1")
```

Both evaluators agree. Public engine wraps the internal error as
Rejected/Internal; printing the successful raw Uuid result separately gives
PrintError. A16-byte control succeeds, prints uuid("61626364-6566-6768-696a-6b6c6d6e6f70")
and text returns the bare lowercase hyphenated string.

A manually constructed generic Form::Newtype with name Uuid and Bytes base
prints/parses byte lengths0,1,16,17 losslessly as Uuid(0x...). Actual Form::Uuid
rejects that spelling at byte0, expected uuid. Therefore changing only printing
would break roundtrip; parsing must admit the fallback too. Existing textual
names are case-sensitive, so uuid and Uuid are distinct, with no lexer addition.
Generic to_text strips the newtype wrapper, so a typed fallback must explicitly
use its print spelling rather than delegate to generic to_text.

Contracts: constraints.md2.3 and prelude.md3.5 require newtype predicates only at
relation writes. Prelude2.7 defines Uuid as new Bytes where bytes_len(self)==16,
and promises text(Uuid) no fault in fixed UUID notation. Interop1.1 and
design/kernel9 require total lossless canonical print/parse for every value.
Those promises conflict outside the write-time domain. Existing
adtb-lang/tests/textual.rs a_value_not_of_its_form_does_not_print and engine
exec/tests.rs printing_a_value_not_of_its_form_is_an_internal_error encode the
length-as-type assumption; they must be corrected under the chosen contract,
not treated as proof that the currently constructible value is mistyped.

Recommendation: retain the newtype and write-only domain, keep existing notation
for16bytes, and use generic Uuid(0x...) for other lengths in print and text.
Accept both wrappers in Uuid.parse, canonicalizing16bytes on printing. Never pad,
truncate, admit an invalid stored UUID, or execute write predicates in the type
checker/constructor/assertion boundary. A fixed-size intrinsic Uuid is a possible
alternative but changes the adopted newtype representation. A formatter fault
preserves the bug in the universal print guarantee and changes the total text API.
