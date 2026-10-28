use std::collections::BTreeMap;

use super::frame::{Frame, Kind, StreamId};
use super::{Error, Result, limit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageLimits {
    pub message_bytes: usize,
    pub buffered_bytes: usize,
    pub partial_streams: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub stream: StreamId,
    pub kind: Kind,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct Reassembler {
    limits: MessageLimits,
    partial: BTreeMap<StreamId, Message>,
    buffered: usize,
}

impl Reassembler {
    pub fn new(limits: MessageLimits) -> Self {
        Self {
            limits,
            partial: BTreeMap::new(),
            buffered: 0,
        }
    }

    /// Append a validated, uncompressed frame, leaving state unchanged on error.
    /// Precondition: after decompression, clear compressed and set length to the decoded size.
    /// Retain other streams when a caller discards a malformed message's stream.
    pub fn push(&mut self, frame: Frame) -> Result<Option<Message>> {
        let header = frame.header;
        if header.compressed {
            return Err(Error::CompressedFrame);
        }
        if u64::from(header.length) != frame.body.len() as u64 {
            return Err(Error::LengthMismatch {
                declared: header.length,
                actual: frame.body.len(),
            });
        }
        let prior = self.partial.get(&header.stream);
        if let Some(prior) = prior
            && prior.kind != header.kind
        {
            return Err(Error::KindChanged {
                stream: header.stream.0,
                expected: prior.kind as u8,
                actual: header.kind as u8,
            });
        }
        let length = prior
            .map_or(0, |p| p.body.len())
            .checked_add(frame.body.len())
            .ok_or(Error::Overflow)?;
        limit("message", length, self.limits.message_bytes)?;
        let total = self
            .buffered
            .checked_add(frame.body.len())
            .ok_or(Error::Overflow)?;
        limit("buffered messages", total, self.limits.buffered_bytes)?;
        if header.more {
            if prior.is_none() {
                limit(
                    "partial streams",
                    self.partial.len() + 1,
                    self.limits.partial_streams,
                )?;
            }
            self.partial
                .entry(header.stream)
                .or_insert_with(|| Message {
                    stream: header.stream,
                    kind: header.kind,
                    body: Vec::new(),
                })
                .body
                .extend(frame.body);
            self.buffered = total;
            Ok(None)
        } else {
            let mut message = self
                .partial
                .remove(&header.stream)
                .unwrap_or_else(|| Message {
                    stream: header.stream,
                    kind: header.kind,
                    body: Vec::new(),
                });
            self.buffered -= message.body.len();
            message.body.extend(frame.body);
            Ok(Some(message))
        }
    }

    pub fn discard(&mut self, stream: StreamId) {
        if let Some(message) = self.partial.remove(&stream) {
            self.buffered -= message.body.len();
        }
    }

    pub fn buffered_bytes(&self) -> usize {
        self.buffered
    }
    pub fn partial_streams(&self) -> usize {
        self.partial.len()
    }

    pub fn finish(&self) -> Result<()> {
        if self.partial.is_empty() {
            Ok(())
        } else {
            Err(Error::Incomplete { needed: 1 })
        }
    }
}
