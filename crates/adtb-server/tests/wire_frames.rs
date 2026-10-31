extern crate adtb_lang as _;

use adtb_server::wire::frame::{
    Chunks, Decoder, Direction, FAIR_CHUNK, Frame, Header, Kind, StreamId,
};
use adtb_server::wire::{Error, Message, MessageLimits, Reassembler};

fn frame(stream: u32, kind: Kind, body: &[u8], more: bool) -> Frame {
    Frame {
        header: Header {
            length: u32::try_from(body.len()).unwrap(),
            stream: StreamId(stream),
            kind,
            more,
            compressed: false,
        },
        body: body.to_vec(),
    }
}

fn encoded(frame: &Frame, direction: Direction) -> Vec<u8> {
    let mut bytes = frame.header.encode(direction, 100_000).unwrap().to_vec();
    bytes.extend(&frame.body);
    bytes
}

fn limits() -> MessageLimits {
    MessageLimits {
        message_bytes: 8,
        buffered_bytes: 12,
        partial_streams: 2,
    }
}

#[test]
fn header_golden_and_invalid_fields() {
    let bytes = [3, 0, 0, 0, 4, 3, 2, 1, 0x10, 3, 0, 0];
    let header = Header {
        length: 3,
        stream: StreamId(0x0102_0304),
        kind: Kind::Call,
        more: true,
        compressed: true,
    };
    assert_eq!(Header::decode(bytes, Direction::Client, 3), Ok(header));
    assert_eq!(header.encode(Direction::Client, 3), Ok(bytes));
    assert_eq!(
        Header::decode(bytes, Direction::Client, 2),
        Err(Error::Limit {
            resource: "frame",
            actual: 3,
            max: 2
        })
    );
    for (index, value, error) in [
        (9, 4, Error::InvalidFlags(4)),
        (10, 1, Error::Reserved(1)),
        (11, 1, Error::Reserved(256)),
        (8, 0x80, Error::UnknownKind(0x80)),
    ] {
        let mut invalid = bytes;
        invalid[index] = value;
        assert_eq!(Header::decode(invalid, Direction::Client, 3), Err(error));
    }
    assert_eq!(
        Header::decode(bytes, Direction::Server, 3),
        Err(Error::WrongDirection(0x10))
    );
}

#[test]
fn all_kinds_obey_direction_and_stream_class() {
    let connection = [1, 2, 3, 6, 0x40, 0x41, 0x42, 0x43];
    let known = [
        1, 2, 3, 4, 5, 6, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x40, 0x41,
        0x42, 0x43, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    ];
    for value in 0..=255 {
        if !known.contains(&value) {
            assert_eq!(Kind::try_from(value), Err(Error::UnknownKind(value)));
            continue;
        }
        let kind = Kind::try_from(value).unwrap();
        let direction = if value < 0x40 {
            Direction::Client
        } else {
            Direction::Server
        };
        let stream = u32::from(!connection.contains(&value));
        let header = Header {
            length: 0,
            stream: StreamId(stream),
            kind,
            more: false,
            compressed: false,
        };
        assert_eq!(header.encode(direction, 0).map(|b| b[8]), Ok(value));
        let other = if direction == Direction::Client {
            Direction::Server
        } else {
            Direction::Client
        };
        if kind == Kind::Credit {
            assert_eq!(header.encode(other, 0).map(|b| b[8]), Ok(value));
        } else {
            assert_eq!(header.encode(other, 0), Err(Error::WrongDirection(value)));
        }
        let wrong = Header {
            stream: StreamId(1 - stream),
            ..header
        };
        if kind == Kind::Error {
            assert_eq!(wrong.encode(direction, 0).map(|b| b[8]), Ok(value));
        } else {
            assert_eq!(
                wrong.encode(direction, 0),
                Err(Error::WrongStream {
                    kind: value,
                    stream: 1 - stream
                })
            );
        }
    }
}

#[test]
fn incremental_decoder_accepts_every_split_and_coalesced_frames() {
    let first = frame(1, Kind::Rows, b"abcdef", false);
    let second = frame(2, Kind::End, b"", false);
    let bytes = encoded(&first, Direction::Server);
    for split in 0..bytes.len() {
        let mut decoder = Decoder::new(Direction::Server, 16);
        let mut prefix = &bytes[..split];
        assert_eq!(decoder.next(&mut prefix), Ok(None), "split {split}");
        assert_eq!(prefix, []);
        let mut rest = &bytes[split..];
        assert_eq!(
            decoder.next(&mut rest),
            Ok(Some(first.clone())),
            "split {split}"
        );
        assert_eq!(rest, []);
        assert_eq!(decoder.finish(), Ok(()));
    }
    let mut coalesced = bytes;
    coalesced.extend(encoded(&second, Direction::Server));
    let mut input = coalesced.as_slice();
    let mut decoder = Decoder::new(Direction::Server, 16);
    assert_eq!(decoder.next(&mut input), Ok(Some(first.clone())));
    assert_eq!(input.len(), 12);
    assert_eq!(decoder.next(&mut input), Ok(Some(second)));
    assert_eq!(input, []);
    assert_eq!(decoder.finish(), Ok(()));
    let bytes = encoded(&first, Direction::Server);
    let mut decoder = Decoder::new(Direction::Server, 16);
    let mut decoded = None;
    for byte in bytes.chunks(1) {
        decoded = decoder.next(&mut &byte[..]).unwrap();
    }
    assert_eq!(decoded, Some(first));
}

#[test]
fn decoder_eof_poison_reset_and_length_refusal() {
    let bytes = encoded(&frame(1, Kind::Call, b"abc", false), Direction::Client);
    for end in 1..bytes.len() {
        let mut decoder = Decoder::new(Direction::Client, 3);
        assert_eq!(decoder.next(&mut &bytes[..end]), Ok(None));
        let needed = if end < 12 {
            12 - end
        } else {
            bytes.len() - end
        };
        assert_eq!(decoder.finish(), Err(Error::Incomplete { needed }));
    }
    let mut invalid = bytes.clone();
    invalid[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut input = invalid.as_slice();
    let mut decoder = Decoder::new(Direction::Client, 3);
    assert_eq!(
        decoder.next(&mut input),
        Err(Error::Limit {
            resource: "frame",
            actual: u64::from(u32::MAX),
            max: 3
        })
    );
    assert_eq!(input, b"abc");
    assert_eq!(decoder.finish(), Err(Error::DecoderFailed));
    assert_eq!(decoder.next(&mut input), Err(Error::DecoderFailed));
    assert_eq!(input, b"abc");
    decoder.reset();
    assert_eq!(decoder.finish(), Ok(()));
    assert_eq!(
        decoder.next(&mut bytes.as_slice()).unwrap().unwrap().body,
        b"abc"
    );
}

#[test]
fn chunks_bound_fairness_and_preserve_empty_messages() {
    let body = vec![0x41; FAIR_CHUNK as usize * 2 + 1];
    let chunks: Vec<_> = Chunks::new(StreamId(1), Kind::Rows, &body, Direction::Server, 100_000)
        .unwrap()
        .collect();
    assert_eq!(
        chunks.iter().map(|f| f.header.length).collect::<Vec<_>>(),
        [65536, 65536, 1]
    );
    assert_eq!(
        chunks.iter().map(|f| f.header.more).collect::<Vec<_>>(),
        [true, true, false]
    );
    assert_eq!(
        chunks
            .iter()
            .flat_map(|f| f.body.iter().copied())
            .collect::<Vec<_>>(),
        body
    );
    assert_eq!(
        Chunks::new(StreamId(1), Kind::Rows, b"abcde", Direction::Server, 2)
            .unwrap()
            .map(|f| f.body)
            .collect::<Vec<_>>(),
        [b"ab".to_vec(), b"cd".to_vec(), b"e".to_vec()]
    );
    assert_eq!(
        Chunks::new(StreamId(1), Kind::End, b"", Direction::Server, 1)
            .unwrap()
            .collect::<Vec<_>>(),
        [frame(1, Kind::End, b"", false)]
    );
    assert!(matches!(
        Chunks::new(StreamId(1), Kind::Rows, b"", Direction::Server, 0),
        Err(Error::Limit {
            resource: "chunk size",
            actual: 1,
            max: 0
        })
    ));
}

#[test]
fn interleaved_messages_discard_and_finish() {
    let mut receiver = Reassembler::new(limits());
    assert_eq!(receiver.push(frame(1, Kind::Rows, b"ab", true)), Ok(None));
    assert_eq!(
        receiver.push(frame(2, Kind::ArrowBatch, b"12", true)),
        Ok(None)
    );
    assert_eq!(receiver.buffered_bytes(), 4);
    assert_eq!(receiver.partial_streams(), 2);
    assert_eq!(receiver.finish(), Err(Error::Incomplete { needed: 1 }));
    assert_eq!(
        receiver.push(frame(1, Kind::Rows, b"cd", false)),
        Ok(Some(Message {
            stream: StreamId(1),
            kind: Kind::Rows,
            body: b"abcd".to_vec()
        }))
    );
    assert_eq!(receiver.buffered_bytes(), 2);
    receiver.discard(StreamId(2));
    receiver.discard(StreamId(99));
    assert_eq!(receiver.buffered_bytes(), 0);
    assert_eq!(receiver.partial_streams(), 0);
    assert_eq!(receiver.finish(), Ok(()));
    assert_eq!(
        receiver.push(frame(1, Kind::End, b"", false)),
        Ok(Some(Message {
            stream: StreamId(1),
            kind: Kind::End,
            body: vec![]
        }))
    );
}

#[test]
fn reassembly_rejects_kind_compression_and_length_without_losing_state() {
    let mut receiver = Reassembler::new(limits());
    assert_eq!(receiver.push(frame(1, Kind::Rows, b"ab", true)), Ok(None));
    assert_eq!(
        receiver.push(frame(1, Kind::End, b"cd", false)),
        Err(Error::KindChanged {
            stream: 1,
            expected: 0x51,
            actual: 0x54
        })
    );
    let mut bad = frame(1, Kind::Rows, b"cd", false);
    bad.header.compressed = true;
    assert_eq!(receiver.push(bad), Err(Error::CompressedFrame));
    for actual in [1, 3] {
        let mut bad = frame(1, Kind::Rows, &vec![0; actual], false);
        bad.header.length = 2;
        assert_eq!(
            receiver.push(bad),
            Err(Error::LengthMismatch {
                declared: 2,
                actual
            })
        );
    }
    assert_eq!(receiver.buffered_bytes(), 2);
    assert_eq!(receiver.partial_streams(), 1);
    assert_eq!(
        receiver
            .push(frame(1, Kind::Rows, b"cd", false))
            .unwrap()
            .unwrap()
            .body,
        b"abcd"
    );
    assert_eq!(receiver.finish(), Ok(()));
}

#[test]
fn reassembly_enforces_message_aggregate_and_stream_limits_atomically() {
    let mut receiver = Reassembler::new(limits());
    assert_eq!(
        receiver.push(frame(1, Kind::Rows, b"123456", true)),
        Ok(None)
    );
    assert_eq!(
        receiver.push(frame(1, Kind::Rows, b"789", false)),
        Err(Error::Limit {
            resource: "message",
            actual: 9,
            max: 8
        })
    );
    assert_eq!(
        receiver.push(frame(2, Kind::Rows, b"abcdef", true)),
        Ok(None)
    );
    assert_eq!(
        receiver.push(frame(1, Kind::Rows, b"7", false)),
        Err(Error::Limit {
            resource: "buffered messages",
            actual: 13,
            max: 12
        })
    );
    assert_eq!(
        receiver.push(frame(3, Kind::Rows, b"", true)),
        Err(Error::Limit {
            resource: "partial streams",
            actual: 3,
            max: 2
        })
    );
    assert_eq!(receiver.buffered_bytes(), 12);
    assert_eq!(receiver.partial_streams(), 2);
    receiver.discard(StreamId(2));
    assert_eq!(
        receiver
            .push(frame(1, Kind::Rows, b"78", false))
            .unwrap()
            .unwrap()
            .body,
        b"12345678"
    );
    assert_eq!(receiver.buffered_bytes(), 0);
    assert_eq!(receiver.finish(), Ok(()));
    let mut receiver = Reassembler::new(MessageLimits {
        partial_streams: 0,
        ..limits()
    });
    assert_eq!(
        receiver.push(frame(1, Kind::Rows, b"", true)),
        Err(Error::Limit {
            resource: "partial streams",
            actual: 1,
            max: 0
        })
    );
    assert_eq!(
        receiver.push(frame(1, Kind::Rows, b"123456789", false)),
        Err(Error::Limit {
            resource: "message",
            actual: 9,
            max: 8
        })
    );
}
