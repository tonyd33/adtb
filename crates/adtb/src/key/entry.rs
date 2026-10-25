use super::codec::{DecodeError, KeySchema, Reader};
use super::ovarint;
use super::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CtorId(pub u32);

/// A row version's address: a 48-bit page id and a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tid {
    page: u64,
    slot: u16,
}

impl Tid {
    pub const PAGE_LIMIT: u64 = 1 << 48;

    /// Returns `None` when `page` does not fit in 48 bits.
    pub fn new(page: u64, slot: u16) -> Option<Tid> {
        (page < Tid::PAGE_LIMIT).then_some(Tid { page, slot })
    }

    pub fn page(self) -> u64 {
        self.page
    }

    pub fn slot(self) -> u16 {
        self.slot
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: Vec<Value>,
    pub ctor: CtorId,
    pub tid: Tid,
}

impl KeySchema {
    pub fn encode_entry(&self, entry: &Entry) -> Vec<u8> {
        let mut out = self.encode(&entry.key);
        ovarint::encode(entry.ctor.0, &mut out);
        out.extend_from_slice(&entry.tid.page.to_be_bytes()[2..]);
        out.extend_from_slice(&entry.tid.slot.to_be_bytes());
        out
    }

    pub fn decode_entry(&self, bytes: &[u8]) -> Result<Entry, DecodeError> {
        let mut r = Reader::new(bytes);
        let key = self.decode_from(&mut r)?;
        let ctor = CtorId(ovarint::decode(&mut r)?);
        let mut page = [0; 8];
        for b in &mut page[2..] {
            *b = r.byte()?;
        }
        let slot = u16::from_be_bytes(r.array()?);
        r.finish()?;
        let tid = Tid {
            page: u64::from_be_bytes(page),
            slot,
        };
        Ok(Entry { key, ctor, tid })
    }
}
