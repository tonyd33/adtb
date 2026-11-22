const P1: u64 = 0x9E37_79B1_85EB_CA87;
const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const P3: u64 = 0x1656_67B1_9E37_79F9;
const P4: u64 = 0x85EB_CA77_C2B2_AE63;
const P5: u64 = 0x27D4_EB2F_1656_67C5;

/// Returns the persisted hash of a key encoding: XXH64 with seed 0.
pub fn hash64(key: &[u8]) -> u64 {
    xxh64(key, 0)
}

/// Returns the high 32 bits of `hash64`.
pub fn hash32(key: &[u8]) -> u32 {
    (hash64(key) >> 32) as u32
}

fn round(acc: u64, lane: u64) -> u64 {
    acc.wrapping_add(lane.wrapping_mul(P2))
        .rotate_left(31)
        .wrapping_mul(P1)
}

fn merge(h: u64, acc: u64) -> u64 {
    (h ^ round(0, acc)).wrapping_mul(P1).wrapping_add(P4)
}

fn xxh64(input: &[u8], seed: u64) -> u64 {
    let (stripes, mut rest) = input.as_chunks::<32>();
    let mut h = if input.len() >= 32 {
        let mut acc = [
            seed.wrapping_add(P1).wrapping_add(P2),
            seed.wrapping_add(P2),
            seed,
            seed.wrapping_sub(P1),
        ];
        for stripe in stripes {
            for (a, lane) in acc.iter_mut().zip(stripe.as_chunks::<8>().0) {
                *a = round(*a, u64::from_le_bytes(*lane));
            }
        }
        let h = acc[0]
            .rotate_left(1)
            .wrapping_add(acc[1].rotate_left(7))
            .wrapping_add(acc[2].rotate_left(12))
            .wrapping_add(acc[3].rotate_left(18));
        acc.iter().fold(h, |h, &a| merge(h, a))
    } else {
        seed.wrapping_add(P5)
    };
    h = h.wrapping_add(input.len() as u64);
    while let Some((lane, tail)) = rest.split_first_chunk::<8>() {
        h ^= round(0, u64::from_le_bytes(*lane));
        h = h.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        rest = tail;
    }
    if let Some((lane, tail)) = rest.split_first_chunk::<4>() {
        h ^= u64::from(u32::from_le_bytes(*lane)).wrapping_mul(P1);
        h = h.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        rest = tail;
    }
    for &b in rest {
        h ^= u64::from(b).wrapping_mul(P5);
        h = h.rotate_left(11).wrapping_mul(P1);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^ (h >> 32)
}
