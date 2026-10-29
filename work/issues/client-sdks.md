---
created-at: 2026-10-28
status: open
kind: feature
priority: 1050
summary: "no SDK lets a TypeScript, Python, Go or Java program connect to an adtb server or call a bundle through typed generated code"
depends-on:
  - milestone-10-server
tags:
  - engine
  - test
---

# Feature: Client SDKs for popular languages

## Request

## Proposal

## Open questions

## Report

adtb has typed code generation for Rust only (`crates/adtb-codegen`, in process). A program in another language has no client for the wire protocol and no generated calls.

## Scope

For TypeScript, Python, Go and Java, and for the existing Rust generator retargeted to the wire:

- a wire client: connect, handshake and authentication, calls and transactions with the retry contract, ad hoc text where granted, feeds and pushes, and errors as typed values;
- a generator that turns a bundle file into typed calls in that language, taking ids and ordinals from `adtb_lang::signature` (docs/design/kernel.md section 8) so generated code agrees with every database built from the same log;
- the wire parity tests (wire.md section 14) and an end-to-end test against a real `adtb-server`, run in CI.

Embedded bindings for other languages through a C ABI over the in-process API are a separate later item.

## Dependencies

Each SDK starts once milestone 10's first authenticated server path passes its parity and lifecycle checks. The languages proceed in parallel after that. Which languages beyond these four, and their packaging, are decided when the track starts.
