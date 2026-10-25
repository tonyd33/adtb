const POLY: u32 = 0x82F6_3B78;

const TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 == 1 { (c >> 1) ^ POLY } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// A running CRC-32C (Castagnoli) over the bytes fed to it.
#[derive(Clone, Copy, Debug)]
pub struct Crc32c(u32);

impl Crc32c {
    pub fn new() -> Crc32c {
        Crc32c(!0)
    }

    pub fn update(mut self, bytes: &[u8]) -> Crc32c {
        for &b in bytes {
            self.0 = TABLE[((self.0 ^ u32::from(b)) & 0xff) as usize] ^ (self.0 >> 8);
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
