use unicode_normalization::is_nfc;

use super::{Error, Result, limit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub epoch: u32,
    pub lsn: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    pub number: u64,
    pub hash: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub bytes: usize,
    pub items: usize,
}

pub type Options<'a> = Vec<(u64, &'a [u8])>;

#[derive(Clone, Debug)]
pub struct Reader<'a> {
    input: &'a [u8],
    limits: Limits,
}

impl<'a> Reader<'a> {
    pub fn new(input: &'a [u8], limits: Limits) -> Self {
        Self { input, limits }
    }

    /// Returns the unconsumed bytes.
    pub fn remaining(&self) -> &'a [u8] {
        self.input
    }

    pub fn finish(self) -> Result<()> {
        if self.input.is_empty() {
            Ok(())
        } else {
            Err(Error::TrailingBytes(self.input.len()))
        }
    }

    /// Read atomically, retaining the cursor when the operation fails.
    pub fn atom<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let mut next = self.clone();
        let value = read(&mut next)?;
        self.input = next.input;
        Ok(value)
    }

    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if count > self.input.len() {
            return Err(Error::Incomplete {
                needed: count - self.input.len(),
            });
        }
        let (value, rest) = self.input.split_at(count);
        self.input = rest;
        Ok(value)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        bytes.copy_from_slice(self.take(N)?);
        Ok(bytes)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.fixed()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.fixed()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.fixed()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.fixed()?))
    }

    pub fn uvarint(&mut self) -> Result<u64> {
        self.atom(|reader| {
            let mut value = 0;
            for index in 0..10 {
                let byte = reader.u8()?;
                if index == 9 && byte > 1 {
                    return Err(Error::Overflow);
                }
                value |= u64::from(byte & 127) << (index * 7);
                if byte & 128 == 0 {
                    if index > 0 && byte == 0 {
                        return Err(Error::NonCanonicalVarint);
                    }
                    return Ok(value);
                }
            }
            Err(Error::Overflow)
        })
    }

    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        self.atom(|reader| {
            let length = usize::try_from(reader.uvarint()?).map_err(|_| Error::Overflow)?;
            limit("bytes", length, reader.limits.bytes)?;
            reader.take(length)
        })
    }

    pub fn text(&mut self) -> Result<&'a str> {
        self.atom(|reader| {
            let value = std::str::from_utf8(reader.bytes()?).map_err(|_| Error::InvalidUtf8)?;
            if !is_nfc(value) {
                return Err(Error::NonNfcText);
            }
            Ok(value)
        })
    }

    pub fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        self.atom(|reader| match reader.u8()? {
            0 => Ok(None),
            1 => read(reader).map(Some),
            tag => Err(Error::InvalidOption(tag)),
        })
    }

    pub fn list<T>(&mut self, mut read: impl FnMut(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        self.atom(|reader| {
            let count = usize::try_from(reader.uvarint()?).map_err(|_| Error::Overflow)?;
            limit("items", count, reader.limits.items)?;
            let mut values = Vec::new();
            for _ in 0..count {
                values.push(read(reader)?);
            }
            Ok(values)
        })
    }

    pub fn options(&mut self, known: &[u64]) -> Result<Options<'a>> {
        let mut previous = None;
        self.list(|reader| {
            let key = reader.uvarint()?;
            if let Some(prior) = previous
                && key <= prior
            {
                return Err(Error::OptionOrder {
                    previous: prior,
                    actual: key,
                });
            }
            if !known.contains(&key) {
                return Err(Error::UnknownOption(key));
            }
            previous = Some(key);
            Ok((key, reader.bytes()?))
        })
    }

    pub fn position(&mut self) -> Result<Position> {
        self.atom(|reader| {
            Ok(Position {
                epoch: reader.u32()?,
                lsn: reader.u64()?,
            })
        })
    }

    pub fn version(&mut self) -> Result<Version> {
        self.atom(|reader| {
            Ok(Version {
                number: reader.u64()?,
                hash: reader.fixed()?,
            })
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
    pub fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    pub fn u16(&mut self, value: u16) {
        self.0.extend(value.to_le_bytes());
    }
    pub fn u32(&mut self, value: u32) {
        self.0.extend(value.to_le_bytes());
    }
    pub fn u64(&mut self, value: u64) {
        self.0.extend(value.to_le_bytes());
    }
    pub fn i32(&mut self, value: i32) {
        self.0.extend(value.to_le_bytes());
    }
    pub fn i64(&mut self, value: i64) {
        self.0.extend(value.to_le_bytes());
    }

    pub fn uvarint(&mut self, mut value: u64) {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            self.u8(if value == 0 { byte } else { byte | 128 });
            if value == 0 {
                break;
            }
        }
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.uvarint(value.len() as u64);
        self.0.extend(value);
    }

    pub fn text(&mut self, value: &str) -> Result<()> {
        if !is_nfc(value) {
            return Err(Error::NonNfcText);
        }
        self.bytes(value.as_bytes());
        Ok(())
    }

    pub fn option<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
        self.u8(u8::from(value.is_some()));
        if let Some(value) = value {
            write(self, value);
        }
    }

    pub fn list<T>(&mut self, values: &[T], mut write: impl FnMut(&mut Self, &T)) {
        self.uvarint(values.len() as u64);
        for value in values {
            write(self, value);
        }
    }

    pub fn position(&mut self, value: Position) {
        self.u32(value.epoch);
        self.u64(value.lsn);
    }
    pub fn version(&mut self, value: Version) {
        self.u64(value.number);
        self.0.extend(value.hash);
    }
}
