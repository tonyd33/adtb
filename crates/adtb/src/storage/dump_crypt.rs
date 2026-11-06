//! The sealing of an encrypted dump: a key derived from a passphrase by Argon2id, and each entry
//! a STREAM of XChaCha20-Poly1305 chunks under a nonce prefix of its own.

use std::fmt;

use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::aead::{KeyInit, Payload};
use zeroize::Zeroizing;

use crate::runtime::Entropy;

/// The plaintext bytes of every chunk of an entry but its last.
const CHUNK: usize = 64 * 1024;
const TAG: usize = 16;
/// XChaCha20's 24-byte nonce less STREAM's 32-bit counter and last-chunk flag.
const PREFIX: usize = 19;
pub const SALT: usize = 16;

/// A passphrase, zeroed on drop. Its `Debug` prints none of its text, and it has no `Display`.
pub struct Passphrase(Zeroizing<String>);

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(..)")
    }
}

impl Passphrase {
    pub fn new(text: String) -> Passphrase {
        Passphrase(Zeroizing::new(text))
    }
}

/// The Argon2id parameters a key is derived under: memory in KiB, passes and lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kdf {
    pub m: u32,
    pub t: u32,
    pub p: u32,
}

impl Kdf {
    pub const DUMP: Kdf = Kdf {
        m: 65536,
        t: 3,
        p: 4,
    };

    /// Returns whether a restore derives a key under these parameters: at most four times the
    /// memory of [`Kdf::DUMP`], about three times its passes and four times its lanes, the most a
    /// crafted manifest may make a restore spend.
    fn bounded(self) -> bool {
        self.m <= 262_144 && self.t <= 10 && self.p <= 16
    }

    /// Returns the parameters the manifest line `kdf argon2id m t p` states, when they are
    /// [`Kdf::bounded`].
    pub fn read(line: &str) -> Option<Kdf> {
        let mut words = line
            .strip_prefix("kdf argon2id ")?
            .split(' ')
            .map(str::parse);
        let kdf = Kdf {
            m: words.next()?.ok()?,
            t: words.next()?.ok()?,
            p: words.next()?.ok()?,
        };
        (words.next().is_none() && kdf.bounded()).then_some(kdf)
    }
}

pub(super) struct DumpKey(XChaCha20Poly1305);

impl DumpKey {
    /// Returns the key `phrase` derives under `kdf` and `salt`, or `None` when Argon2id refuses
    /// the parameters.
    pub fn derive(phrase: &Passphrase, kdf: Kdf, salt: &[u8; SALT]) -> Option<DumpKey> {
        let params = Params::new(kdf.m, kdf.t, kdf.p, Some(32)).ok()?;
        let mut memory = Zeroizing::new(vec![Block::default(); params.block_count()]);
        let mut key = Zeroizing::new([0; 32]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into_with_memory(phrase.0.as_bytes(), salt, &mut *key, &mut *memory)
            .ok()?;
        Some(DumpKey(XChaCha20Poly1305::new((&*key).into())))
    }

    /// Returns `plain` sealed under a fresh nonce prefix drawn from `rt`, each chunk
    /// authenticating `ad`.
    pub fn seal(&self, rt: &impl Entropy, ad: &[u8], plain: &[u8]) -> Vec<u8> {
        let mut prefix = [0; PREFIX];
        rt.fill(&mut prefix);
        let mut stream = EncryptorBE32::from_aead(self.0.clone(), (&prefix).into());
        let last = plain.len().saturating_sub(1) / CHUNK * CHUNK;
        let mut out = prefix.to_vec();
        let full = "an entry holds fewer than 2^32 chunks";
        for msg in plain[..last].chunks(CHUNK) {
            out.extend(stream.encrypt_next(Payload { msg, aad: ad }).expect(full));
        }
        let msg = &plain[last..];
        out.extend(stream.encrypt_last(Payload { msg, aad: ad }).expect(full));
        out
    }

    /// Returns what [`DumpKey::seal`] sealed as `sealed` under `ad`, or `None` on a chunk failing
    /// to authenticate or on chunks ending before a last one.
    pub fn open(&self, ad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
        let (prefix, body) = sealed.split_at_checked(PREFIX)?;
        let mut stream = DecryptorBE32::from_aead(self.0.clone(), prefix.into());
        let whole = CHUNK + TAG;
        let last = body.len().checked_sub(1)? / whole * whole;
        let mut plain = Vec::with_capacity(body.len());
        for msg in body[..last].chunks(whole) {
            plain.extend(stream.decrypt_next(Payload { msg, aad: ad }).ok()?);
        }
        let msg = &body[last..];
        plain.extend(stream.decrypt_last(Payload { msg, aad: ad }).ok()?);
        Some(plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter(std::sync::atomic::AtomicU64);

    impl Entropy for Counter {
        fn next_u64(&self) -> u64 {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        }
    }

    fn key(phrase: &str) -> DumpKey {
        let kdf = Kdf { m: 8, t: 1, p: 1 };
        DumpKey::derive(&Passphrase::new(phrase.to_owned()), kdf, &[7; SALT]).unwrap()
    }

    fn sealed(len: usize) -> (Vec<u8>, Vec<u8>) {
        let plain: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        let sealed = key("k").seal(&Counter(0.into()), b"data/a", &plain);
        (plain, sealed)
    }

    #[test]
    fn an_entry_opens_whole_across_chunk_boundaries() {
        for len in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 2 * CHUNK + 5] {
            let (plain, sealed) = sealed(len);
            let chunks = len.saturating_sub(1) / CHUNK + 1;
            assert_eq!(PREFIX + len + chunks * TAG, sealed.len(), "{len}");
            assert_eq!(Some(plain), key("k").open(b"data/a", &sealed), "{len}");
        }
    }

    #[test]
    fn another_path_or_key_does_not_open_an_entry() {
        let (_, sealed) = sealed(10);
        assert_eq!(None, key("k").open(b"data/b", &sealed));
        assert_eq!(None, key("j").open(b"data/a", &sealed));
    }

    #[test]
    fn an_entry_cut_at_a_chunk_reordered_or_flipped_does_not_open() {
        let (_, sealed) = sealed(2 * CHUNK + 5);
        let whole = CHUNK + TAG;
        let cut = &sealed[..PREFIX + 2 * whole];
        let mut swapped = sealed.clone();
        swapped[PREFIX..PREFIX + 2 * whole].rotate_left(whole);
        let mut flipped = sealed.clone();
        flipped[PREFIX + whole + 3] ^= 1;
        for (name, bad) in [("cut", cut), ("swapped", &swapped), ("flipped", &flipped)] {
            assert_eq!(None, key("k").open(b"data/a", bad), "{name}");
        }
        assert_eq!(None, key("k").open(b"data/a", &sealed[..PREFIX]));
    }

    #[test]
    fn a_restore_reads_parameters_at_the_limits_and_none_past_any() {
        let at = |m, t, p| Kdf::read(&format!("kdf argon2id {m} {t} {p}"));
        assert_eq!(
            Some(Kdf {
                m: 262_144,
                t: 10,
                p: 16
            }),
            at(262_144, 10, 16)
        );
        assert_eq!(Some(Kdf::DUMP), at(65536, 3, 4));
        for (m, t, p) in [(262_145, 10, 16), (262_144, 11, 16), (262_144, 10, 17)] {
            assert_eq!(None, at(m, t, p), "{m} {t} {p}");
        }
    }

    #[test]
    fn a_printed_passphrase_holds_no_byte_of_it() {
        let text = "XYZ0129!";
        let printed = format!("{:?}", Passphrase::new(text.to_owned()));
        assert_eq!("Passphrase(..)", printed);
        assert!(!printed.bytes().any(|b| text.as_bytes().contains(&b)));
    }
}
