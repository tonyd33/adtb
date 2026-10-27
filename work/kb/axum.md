# axum

kind: system
home: https://github.com/tokio-rs/axum

## licences and dependency tree

- axum 0.8.9 with default features off and `http1`, `tokio`, `form`, `query` resolves, with tokio 1.54.1 (`rt-multi-thread`, `net`, `macros`, `sync`), to about fifty crates: axum, axum-core, hyper 1.12, hyper-util, http, http-body, http-body-util, httparse, httpdate, tower 0.5, tower-layer, tower-service, matchit 0.8.4, mime, bytes, futures-core, futures-task, futures-util, pin-project-lite, sync_wrapper, serde, serde_core, serde_derive, serde_path_to_error, serde_urlencoded, form_urlencoded, percent-encoding, itoa, ryu, memchr, mio, socket2, slab, atomic-waker, libc, tokio-macros. [V 2026-10-26] cargo metadata against crates.io
- Licences: matchit is `MIT AND BSD-3-Clause`; ryu `Apache-2.0 OR BSL-1.0`; memchr `Unlicense OR MIT`; sync_wrapper `Apache-2.0`; the rest MIT or MIT/Apache-2.0. [V 2026-10-26] cargo metadata against crates.io
- serde_derive 1.0.229 and tokio-macros 2.7.2 depend on syn 3; maud_macros 0.27.0 on syn 2. [V 2026-10-26] cargo metadata against crates.io
