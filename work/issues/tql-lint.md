---
created-at: 2026-11-18
status: resolved
closed: 2026-11-18
kind: feature
priority: 24
tags: [tooling]
summary: "house, structure and refusal checks as TQL rules under lint/, run by cargo xtask lint"
---

# Feature: Tql lint

## Request

Ruled with the maintainer in session, 2026-11-18 (work/briefs/tql-session-2026-11-18.md): house-check, structure-check and refusal-check become TQL 0.4.0 rules in lint/*.tql, zero-match or per-file count ratchets in lint/allowlist.toml, run by `cargo xtask lint`; tql pinned as a flake input. The maintainer writes the rules in session with the lead; plumbing is a track.

## 2026-11-18: plumbing landed

`feat: run TQL lint rules with cargo xtask lint`: lint/rules.toml declares runs (query, include, exclude, rows zero or ratchet), lint/allowlist.toml holds ratchet counts, both read with the toml crate (xtask only). tql 0.4.0 is a flake input, version checked at run. house-check deleted; house.tql, structure rows (pub_fields, crate_fields, decode_panics, engine_names, sim_test_opens ratchets; prelude_positions, doc-verb zero) ported. Open on this ticket: gate-rows wiring and deleting structure-check and adtb-structure (slot-4), retiring refusal-check into lint/.

## 2026-11-18: gate rows landed

`refactor: gate quarantine marks and api aborts by type, clippy and TQL, and delete structure-check`: opaque Mark, pub(super) Marks setters, clippy disallows Engine::abort; lint/holder-builders.tql, marked-readers.tql, api-aborts.tql zero rows (known misses in work/briefs/receipt-gate-rows-r1.md); structure-check, its allowlist and test and crates/adtb-structure deleted. Open: refusal rule into lint and tree-sitter-check into xtask (brief-public-checks-r1.md).

## 2026-11-18: public checks landed

`refactor: move the refusal rule into TQL lint and the grammar check into xtask`: lint/refusal.tql is a zero row over crates/ with a fixture test; tree-sitter generate, compare and test is an xtask check stage. refusal-check, its allowlist and tree-sitter-check deleted. The public checks now call nothing under work/tools; remaining mentions are lead-tool instructions. Every rule of the old house-check, structure-check and refusal-check now runs in `cargo xtask lint` or the type system.
