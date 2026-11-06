# RustCrypto

kind: system
home: https://github.com/RustCrypto

## chacha20poly1305

- Version 0.10.1 implements ChaCha20Poly1305 and XChaCha20Poly1305; its `stream` feature enables `aead::stream`, the STREAM construction with `EncryptorBE32` and `DecryptorBE32`. [V 2026-11-05] https://crates.io/crates/chacha20poly1305/0.10.1 (published Cargo.toml and README)
- Licensed Apache-2.0 OR MIT. [V 2026-11-05] https://crates.io/crates/chacha20poly1305/0.10.1
- Audited by NCC Group in 2020 with no significant findings, together with aes-gcm, funded by MobileCoin; the README says so. [V 2026-11-05] https://research.nccgroup.com/2020/02/26/public-report-rustcrypto-aes-gcm-and-chacha20poly1305-implementation-review/ (cited in the crate README)
- Its cipher type zeroes its key on drop and implements `ZeroizeOnDrop`. [V 2026-11-05] chacha20poly1305 0.10.1 src/lib.rs
- The chacha20 0.9.1 and poly1305 0.8.0 crates under it carry the same NCC audit; poly1305's README notes the audit predates its AVX2 backend. [V 2026-11-05] https://crates.io/crates/poly1305/0.8.0 (README)
- 0.11.0 is published. [V 2026-11-05] cargo resolution against crates.io

## aead stream

- In STREAM BE32 the nonce is the cipher's nonce less 5 bytes: a 32-bit big-endian chunk counter and a last-chunk flag byte. For XChaCha20Poly1305 the caller's prefix is 19 bytes. [V 2026-11-05] aead 0.5.2 src/stream.rs
- `encrypt_last` and `decrypt_last` consume the stream, so a chunk sealed as last cannot be followed, and one sealed as not last fails to open as last. [V 2026-11-05] aead 0.5.2 src/stream.rs

## argon2

- Version 0.5.3 implements Argon2d, Argon2i and Argon2id, version 0x13; licensed MIT OR Apache-2.0. [V 2026-11-05] https://crates.io/crates/argon2/0.5.3
- `hash_password_into` needs the `alloc` feature, which pulls `password-hash`; `hash_password_into_with_memory` takes caller memory and needs neither. [V 2026-11-05] argon2 0.5.3 src/lib.rs
- The optional `zeroize` feature zeroes the initial hash and makes `Block` implement `Zeroize`. [V 2026-11-05] argon2 0.5.3 src/lib.rs, src/block.rs
- `Params` accepts m from 8·p KiB up to u32::MAX, t from 1, p from 1 up to 2^24 - 1. [V 2026-11-05] argon2 0.5.3 src/params.rs
- Its README names no security audit. [V 2026-11-05] argon2 0.5.3 README.md
- 0.6.0 is published. [V 2026-11-05] cargo resolution against crates.io
