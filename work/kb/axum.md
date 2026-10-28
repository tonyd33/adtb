# axum

kind: system
home: https://github.com/tokio-rs/axum

## licences and dependency tree

- axum 0.8.9 with default features off and `http1`, `tokio`, `form`, `query` resolves, with tokio 1.54.1 (`rt-multi-thread`, `net`, `macros`, `sync`), to about fifty crates: axum, axum-core, hyper 1.12, hyper-util, http, http-body, http-body-util, httparse, httpdate, tower 0.5, tower-layer, tower-service, matchit 0.8.4, mime, bytes, futures-core, futures-task, futures-util, pin-project-lite, sync_wrapper, serde, serde_core, serde_derive, serde_path_to_error, serde_urlencoded, form_urlencoded, percent-encoding, itoa, ryu, memchr, mio, socket2, slab, atomic-waker, libc, tokio-macros. [V 2026-10-26] cargo metadata against crates.io
- Licences: matchit is `MIT AND BSD-3-Clause`; ryu `Apache-2.0 OR BSL-1.0`; memchr `Unlicense OR MIT`; sync_wrapper `Apache-2.0`; the rest MIT or MIT/Apache-2.0. [V 2026-10-26] cargo metadata against crates.io
- serde_derive 1.0.229 and tokio-macros 2.7.2 depend on syn 3; maud_macros 0.27.0 on syn 2. [V 2026-10-26] cargo metadata against crates.io

## campaign server

- `axum::serve` accepts a Tokio TcpListener and a Router with the `tokio` and `http1` features. [V 2026-10-27] https://docs.rs/axum/0.8.9/axum/fn.serve.html
- `spawn_blocking` runs bounded blocking closures off the async executor; Tokio recommends limiting CPU work with a semaphore, and already started blocking closures cannot be aborted. [V 2026-10-27] https://docs.rs/tokio/1.54.1/tokio/task/fn.spawn_blocking.html
- Campaign resolves axum 0.8.9, tokio 1.54.1, maud 0.27.0 and serde 1.0.229. Tokio `signal` adds signal-hook-registry and errno; maud macros add proc-macro2-diagnostics and version_check. Licenses remain MIT/Apache-compatible except matchit's combined MIT AND BSD-3-Clause. [V 2026-10-27] `cargo metadata --manifest-path apps/campaign/Cargo.toml --format-version=1`, resolved registry package manifests and Cargo.lock
- Correction to the 2026-10-26 design interpretation: having syn 2 and 3 in a lockfile does not permit duplicates under cargo-deny. The campaign's active maud_macros 0.27.0 and proc-macro2-diagnostics 0.10.1 depend on syn 2.0.119, while the ICU, serde and tokio macro branches use syn 3.0.9. `cargo deny --manifest-path apps/campaign/Cargo.toml check` rejects both without a narrow exception. [V 2026-10-27] resolved registry Cargo.toml manifests, `cargo tree --manifest-path apps/campaign/Cargo.toml -i syn@2.0.119`, cargo-deny diagnostic
