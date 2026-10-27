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

/// A running CRC-32C (Castagnoli) over the bytes fed to it.
#[derive(Clone, Copy, Debug)]
pub struct Crc32c(u32);

impl Crc32c {
    pub fn new() -> Crc32c {
        Crc32c(!0)
    }

    pub fn update(mut self, bytes: &[u8]) -> Crc32c {
        let t = &TABLES;
        let (words, rest) = bytes.as_chunks::<8>();
        for w in words {
            let c = self.0 ^ u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
            self.0 = t[7][(c & 0xff) as usize]
                ^ t[6][((c >> 8) & 0xff) as usize]
                ^ t[5][((c >> 16) & 0xff) as usize]
                ^ t[4][(c >> 24) as usize]
                ^ t[3][usize::from(w[4])]
                ^ t[2][usize::from(w[5])]
                ^ t[1][usize::from(w[6])]
                ^ t[0][usize::from(w[7])];
        }
        for &b in rest {
            self.0 = t[0][((self.0 ^ u32::from(b)) & 0xff) as usize] ^ (self.0 >> 8);
        }
        self
    }

    pub fn finish(self) -> u32 {
        !self.0
    }
}

impl Default for Crc32c {
    fn default() -> Self {
        Crc32c::new()
    }
}

pub fn crc32c(bytes: &[u8]) -> u32 {
    Crc32c::new().update(bytes).finish()
}

/// Returns the CRC-32C of `bytes` with the four bytes at `field` read as zero, seeded by `seed`
/// fed first as eight little-endian bytes.
pub fn crc32c_skipping(seed: Option<u64>, bytes: &[u8], field: usize) -> u32 {
    let mut c = Crc32c::new();
    if let Some(seed) = seed {
        c = c.update(&seed.to_le_bytes());
    }
    c.update(&bytes[..field])
        .update(&[0; 4])
        .update(&bytes[field + 4..])
        .finish()
}
