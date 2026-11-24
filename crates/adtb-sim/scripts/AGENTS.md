# Engine scripts

Scripts in the conformance format, run on the engine over the simulator and never on the
reference. The directives are the doc comment of `crates/adtb-conformance/src/scripted/mod.rs`;
`use` reads a fixture from `fixtures/`.

- A refusal is stated with its whole reason in textual syntax, after `error`, `refused`,
  `failed`, `retry` or `fatal`.
- One script per contract, its file named for the contract it checks.
- Each directory but `fixtures/` and `probes/` is an area with a test in
  `crates/adtb-sim/tests/scripted/scripts/mod.rs`, split by `parts`, and an override in
  `.config/nextest.toml` giving that test 3 slots; `every_area_has_its_test_holding_its_slots`
  fails without them.
- `probes/` holds scripts that must differ as the `differs` file beside each states. Every
  directive has a malformed-form probe under `probes/forms/`;
  `every_directive_has_a_malformed_form_probe` fails without it.

Run an area or a file with `cargo conformance --file-name crates/adtb-sim/scripts/execution`;
`--update` writes the rows each differing directive got. As tests:
`cargo nextest run -p adtb-sim --test scripted -E 'test(/^scripts::/)'`.
