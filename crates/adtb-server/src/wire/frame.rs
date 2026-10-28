use super::{Error, Result};

pub const HEADER_LEN: usize = 12;
pub const FAIR_CHUNK: u32 = 65536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Client,
    Server,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Hello = 0x01,
    AuthResponse = 0x02,
    Ping = 0x03,
    Cancel = 0x04,
    Credit = 0x05,
    Goodbye = 0x06,
    Call = 0x10,
    CallText = 0x11,
    Begin = 0x12,
    Commit = 0x13,
    Rollback = 0x14,
    Subscribe = 0x15,
    FeedAck = 0x16,
    Append = 0x17,
    AppendData = 0x18,
    AppendEnd = 0x19,
    AuthChallenge = 0x40,
    Ready = 0x41,
    Pong = 0x42,
    Push = 0x43,
    ResultType = 0x50,
    Rows = 0x51,
    ArrowSchema = 0x52,
    ArrowBatch = 0x53,
    End = 0x54,
    Error = 0x55,
    TxnOpened = 0x56,
    FeedSnapshot = 0x57,
    FeedSnapshotEnd = 0x58,
    FeedChanges = 0x59,
}

impl TryFrom<u8> for Kind {
    type Error = Error;
    fn try_from(value: u8) -> Result<Self> {
        Ok(match value {
            0x01 => Self::Hello,
            0x02 => Self::AuthResponse,
            0x03 => Self::Ping,
            0x04 => Self::Cancel,
            0x05 => Self::Credit,
            0x06 => Self::Goodbye,
            0x10 => Self::Call,
            0x11 => Self::CallText,
            0x12 => Self::Begin,
            0x13 => Self::Commit,
            0x14 => Self::Rollback,
            0x15 => Self::Subscribe,
            0x16 => Self::FeedAck,
            0x17 => Self::Append,
            0x18 => Self::AppendData,
            0x19 => Self::AppendEnd,
            0x40 => Self::AuthChallenge,
            0x41 => Self::Ready,
            0x42 => Self::Pong,
            0x43 => Self::Push,
            0x50 => Self::ResultType,
            0x51 => Self::Rows,
            0x52 => Self::ArrowSchema,
            0x53 => Self::ArrowBatch,
            0x54 => Self::End,
            0x55 => Self::Error,
            0x56 => Self::TxnOpened,
            0x57 => Self::FeedSnapshot,
            0x58 => Self::FeedSnapshotEnd,
            0x59 => Self::FeedChanges,
            _ => return Err(Error::UnknownKind(value)),
        })
    }
}

impl Kind {
    fn validate(self, direction: Direction, stream: StreamId) -> Result<()> {
        if self != Self::Credit && (((self as u8) < 0x40) != (direction == Direction::Client)) {
            return Err(Error::WrongDirection(self as u8));
        }
        let connection = matches!(
            self,
            Self::Hello
                | Self::AuthResponse
                | Self::Ping
                | Self::Goodbye
                | Self::AuthChallenge
                | Self::Ready
                | Self::Pong
                | Self::Push
        );
        if self != Self::Error && connection != (stream.0 == 0) {
            return Err(Error::WrongStream {
                kind: self as u8,
                stream: stream.0,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub length: u32,
    pub stream: StreamId,
    pub kind: Kind,
    pub more: bool,
    pub compressed: bool,
}

impl Header {
    pub fn decode(bytes: [u8; HEADER_LEN], direction: Direction, max_frame: u32) -> Result<Self> {
        let length = u32::from_le_bytes(
            bytes[..4]
                .try_into()
                .expect("header length field is four bytes"),
        );
        if length > max_frame {
            return Err(Error::Limit {
                resource: "frame",
                actual: u64::from(length),
                max: u64::from(max_frame),
            });
        }
        let stream = StreamId(u32::from_le_bytes(
            bytes[4..8]
                .try_into()
                .expect("header stream field is four bytes"),
        ));
        let kind = Kind::try_from(bytes[8])?;
        if bytes[9] & !3 != 0 {
            return Err(Error::InvalidFlags(bytes[9]));
        }
        let reserved = u16::from_le_bytes([bytes[10], bytes[11]]);
        if reserved != 0 {
            return Err(Error::Reserved(reserved));
        }
        kind.validate(direction, stream)?;
        Ok(Self {
            length,
            stream,
            kind,
            more: bytes[9] & 1 != 0,
            compressed: bytes[9] & 2 != 0,
        })
    }

    pub fn encode(self, direction: Direction, max_frame: u32) -> Result<[u8; HEADER_LEN]> {
        let mut bytes = [0; HEADER_LEN];
        bytes[..4].copy_from_slice(&self.length.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.stream.0.to_le_bytes());
        bytes[8] = self.kind as u8;
        bytes[9] = u8::from(self.more) | (u8::from(self.compressed) << 1);
        Self::decode(bytes, direction, max_frame)?;
        Ok(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub header: Header,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct Decoder {
    direction: Direction,
    max_frame: u32,
    bytes: Vec<u8>,
    header: Option<Header>,
    failed: bool,
}

impl Decoder {
    pub fn new(direction: Direction, max_frame: u32) -> Self {
        Self {
            direction,
            max_frame,
            bytes: Vec::new(),
            header: None,
            failed: false,
        }
    }

    /// Consume at most one frame, retaining incomplete input internally.
    /// Return DecoderFailed after a malformed header until reset is called.
    pub fn next(&mut self, input: &mut &[u8]) -> Result<Option<Frame>> {
        if self.failed {
            return Err(Error::DecoderFailed);
        }
        if self.header.is_none() {
            let count = (HEADER_LEN - self.bytes.len()).min(input.len());
            self.bytes.extend_from_slice(&input[..count]);
            *input = &input[count..];
            if self.bytes.len() < HEADER_LEN {
                return Ok(None);
            }
            let bytes = self
                .bytes
                .as_slice()
                .try_into()
                .expect("complete header has twelve bytes");
            match Header::decode(bytes, self.direction, self.max_frame) {
                Ok(header) => {
                    self.header = Some(header);
                    self.bytes.clear();
                }
                Err(error) => {
                    self.failed = true;
                    return Err(error);
                }
            }
        }
        let header = self
            .header
            .expect("body decoding follows a validated header");
        let length = usize::try_from(header.length).map_err(|_| Error::Overflow)?;
        let count = (length - self.bytes.len()).min(input.len());
        self.bytes.extend_from_slice(&input[..count]);
        *input = &input[count..];
        if self.bytes.len() < length {
            return Ok(None);
        }
        self.header = None;
        Ok(Some(Frame {
            header,
            body: std::mem::take(&mut self.bytes),
        }))
    }

    pub fn finish(&self) -> Result<()> {
        if self.failed {
            return Err(Error::DecoderFailed);
        }
        let needed = self.header.map_or(HEADER_LEN, |h| h.length as usize) - self.bytes.len();
        if self.header.is_some() || !self.bytes.is_empty() {
            Err(Error::Incomplete { needed })
        } else {
            Ok(())
        }
    }

    pub fn reset(&mut self) {
        self.bytes = Vec::new();
        self.header = None;
        self.failed = false;
    }
}

#[derive(Debug)]
pub struct Chunks<'a> {
    body: &'a [u8],
    stream: StreamId,
    kind: Kind,
    size: usize,
    finished: bool,
}

impl<'a> Chunks<'a> {
    pub fn new(
        stream: StreamId,
        kind: Kind,
        body: &'a [u8],
        direction: Direction,
        max_frame: u32,
    ) -> Result<Self> {
        kind.validate(direction, stream)?;
        if max_frame == 0 {
            return Err(Error::Limit {
                resource: "chunk size",
                actual: 1,
                max: 0,
            });
        }
        let size = usize::try_from(max_frame.min(FAIR_CHUNK)).map_err(|_| Error::Overflow)?;
        Ok(Self {
            body,
            stream,
            kind,
            size,
            finished: false,
        })
    }
}

impl Iterator for Chunks<'_> {
    type Item = Frame;
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let count = self.body.len().min(self.size);
        let (part, rest) = self.body.split_at(count);
        self.body = rest;
        self.finished = rest.is_empty();
        Some(Frame {
            header: Header {
                length: count as u32,
                stream: self.stream,
                kind: self.kind,
                more: !self.finished,
                compressed: false,
            },
            body: part.to_vec(),
        })
    }
}
