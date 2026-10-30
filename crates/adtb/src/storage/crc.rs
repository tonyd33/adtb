/// A running CRC-32C (Castagnoli) over the bytes fed to it.
#[derive(Clone, Copy, Debug)]
pub struct Crc32c(u32);

impl Crc32c {
    pub fn new() -> Crc32c {
        Crc32c(!0)
    }

    pub fn update(self, bytes: &[u8]) -> Crc32c {
        Crc32c(adtb_sys::crc32c::update(self.0, bytes))
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
