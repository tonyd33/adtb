//! The Castagnoli CRC, by the CPU's instruction where it has one and by table otherwise. Both
//! produce the same bytes. Miri cannot run the instruction, so under Miri only the table runs.

const POLY: u32 = 0x82F6_3B78;

/// `TABLES[k][b]` is the CRC update of byte `b` followed by `k` zero bytes.
const TABLES: [[u32; 256]; 8] = {
    let mut tables = [[0; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 == 1 { (c >> 1) ^ POLY } else { c >> 1 };
            k += 1;
        }
        tables[0][i] = c;
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            let prev = tables[k - 1][i];
            tables[k][i] = (prev >> 8) ^ tables[0][(prev & 0xff) as usize];
            i += 1;
        }
        k += 1;
    }
    tables
};

/// Returns `register` advanced over `bytes`. The register is not inverted here: start from `!0`
/// and invert the last value for the checksum of the bytes.
pub fn update(register: u32, bytes: &[u8]) -> u32 {
    hardware(register, bytes).unwrap_or_else(|| table(register, bytes))
}

fn table(mut register: u32, bytes: &[u8]) -> u32 {
    let t = &TABLES;
    let (words, rest) = bytes.as_chunks::<8>();
    for w in words {
        let c = register ^ u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        register = t[7][(c & 0xff) as usize]
            ^ t[6][((c >> 8) & 0xff) as usize]
            ^ t[5][((c >> 16) & 0xff) as usize]
            ^ t[4][(c >> 24) as usize]
            ^ t[3][usize::from(w[4])]
            ^ t[2][usize::from(w[5])]
            ^ t[1][usize::from(w[6])]
            ^ t[0][usize::from(w[7])];
    }
    for &b in rest {
        register = t[0][((register ^ u32::from(b)) & 0xff) as usize] ^ (register >> 8);
    }
    register
}

/// Returns `None` when the CPU has no crc32c instruction.
#[cfg(all(target_arch = "x86_64", not(miri)))]
fn hardware(register: u32, bytes: &[u8]) -> Option<u32> {
    if !std::is_x86_feature_detected!("sse4.2") {
        return None;
    }
    // SAFETY: the check above found the one feature `sse42` enables.
    Some(unsafe { sse42(register, bytes) })
}

#[cfg(all(target_arch = "x86_64", not(miri)))]
#[target_feature(enable = "sse4.2")]
fn sse42(register: u32, bytes: &[u8]) -> u32 {
    use std::arch::x86_64::{_mm_crc32_u8, _mm_crc32_u64};

    let (words, rest) = bytes.as_chunks::<8>();
    let mut wide = u64::from(register);
    for w in words {
        wide = _mm_crc32_u64(wide, u64::from_le_bytes(*w));
    }
    let mut c = wide as u32;
    for &b in rest {
        c = _mm_crc32_u8(c, b);
    }
    c
}

/// Returns `None` when the CPU has no crc32c instruction.
#[cfg(all(target_arch = "aarch64", not(miri)))]
fn hardware(register: u32, bytes: &[u8]) -> Option<u32> {
    if !std::arch::is_aarch64_feature_detected!("crc") {
        return None;
    }
    // SAFETY: the check above found the one feature `armv8_crc` enables.
    Some(unsafe { armv8_crc(register, bytes) })
}

#[cfg(all(target_arch = "aarch64", not(miri)))]
#[target_feature(enable = "crc")]
fn armv8_crc(register: u32, bytes: &[u8]) -> u32 {
    use std::arch::aarch64::{__crc32cb, __crc32cd};

    let (words, rest) = bytes.as_chunks::<8>();
    let mut c = register;
    for w in words {
        c = __crc32cd(c, u64::from_le_bytes(*w));
    }
    for &b in rest {
        c = __crc32cb(c, b);
    }
    c
}

#[cfg(any(miri, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
fn hardware(_register: u32, _bytes: &[u8]) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::{POLY, hardware, table, update};

    fn bitwise(mut register: u32, bytes: &[u8]) -> u32 {
        for &b in bytes {
            register ^= u32::from(b);
            for _ in 0..8 {
                register = if register & 1 == 1 {
                    (register >> 1) ^ POLY
                } else {
                    register >> 1
                };
            }
        }
        register
    }

    /// Returns the result of each path this build can take.
    fn paths(register: u32, bytes: &[u8]) -> Vec<u32> {
        let mut out = vec![table(register, bytes), update(register, bytes)];
        out.extend(hardware(register, bytes));
        out
    }

    fn fill(seed: u64, out: &mut [u8]) {
        let mut x = seed;
        for b in out {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let z = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            *b = (z >> 56) as u8;
        }
    }

    #[test]
    #[cfg(all(not(miri), any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn the_instruction_runs_on_this_cpu() {
        assert!(hardware(!0, b"x").is_some());
    }

    #[test]
    fn known_answers() {
        let zeros = [0; 32];
        let ones = [0xff; 32];
        let up: Vec<u8> = (0..32).collect();
        let down: Vec<u8> = (0..32).rev().collect();
        let pdu: [u8; 48] = [
            0x01, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x14,
            0x00, 0x00, 0x00, 0x18, 0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let vectors: [(&[u8], u32); 7] = [
            (b"", 0),
            (b"123456789", 0xE306_9283),
            (&zeros, 0x8A91_36AA),
            (&ones, 0x62A8_AB43),
            (&up, 0x46DD_794E),
            (&down, 0x113F_DB5C),
            (&pdu, 0xD996_3A56),
        ];
        for (bytes, want) in vectors {
            for got in paths(!0, bytes) {
                assert_eq!(want, !got, "{bytes:02x?}");
            }
        }
    }

    #[test]
    fn every_path_matches_the_bitwise_definition_over_a_page() {
        let mut page = vec![0; if cfg!(miri) { 256 } else { 4096 }];
        fill(1, &mut page);
        let want = bitwise(!0, &page);
        for got in paths(!0, &page) {
            assert_eq!(want, got);
        }
    }

    #[test]
    fn the_paths_agree_on_every_short_length_and_alignment() {
        let mut bytes = vec![0; if cfg!(miri) { 24 } else { 96 }];
        fill(2, &mut bytes);
        for start in 0..if cfg!(miri) { 8 } else { 16 } {
            for end in start..bytes.len() {
                let part = &bytes[start..end];
                let want = bitwise(0x1234_5678, part);
                for got in paths(0x1234_5678, part) {
                    assert_eq!(want, got, "{start}..{end}");
                }
            }
        }
    }

    #[test]
    fn the_paths_agree_on_random_lengths_and_alignments() {
        let mut bytes = vec![0; if cfg!(miri) { 96 } else { 9000 }];
        let mut places = [0; 3];
        for seed in 0..if cfg!(miri) { 8 } else { 400 } {
            fill(seed, &mut bytes);
            fill(seed ^ 0xAAAA, &mut places);
            let start = usize::from(places[0] % 32);
            let len = (usize::from(places[1]) << 8 | usize::from(places[2])) % (bytes.len() - 32);
            let part = &bytes[start..start + len];
            let got = paths(!0, part);
            assert!(got.iter().all(|&c| c == got[0]), "seed {seed}: {got:x?}");
        }
    }

    #[test]
    fn an_update_split_at_every_offset_changes_nothing() {
        let mut bytes = [0; 40];
        fill(3, &mut bytes);
        let want = bitwise(!0, &bytes);
        for mid in 0..=bytes.len() {
            let (a, b) = bytes.split_at(mid);
            for first in paths(!0, a) {
                for got in paths(first, b) {
                    assert_eq!(want, got, "split at {mid}");
                }
            }
        }
    }
}
