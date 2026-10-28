use adtb_server::wire::Error;
use adtb_server::wire::primitives::{Limits, Position, Reader, Version, Writer};

const LIMITS: Limits = Limits {
    bytes: 1024,
    items: 32,
};

#[test]
fn scalar_golden_bytes() {
    let mut writer = Writer::new();
    writer.u8(0x81);
    writer.u16(0x2345);
    writer.u32(0x6789_abcd);
    writer.u64(0x0123_4567_89ab_cdef);
    writer.i32(-2);
    writer.i64(-3);
    let bytes = writer.into_bytes();
    assert_eq!(
        bytes,
        [
            0x81, 0x45, 0x23, 0xcd, 0xab, 0x89, 0x67, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23,
            0x01, 0xfe, 0xff, 0xff, 0xff, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
        ]
    );
    let mut reader = Reader::new(&bytes, LIMITS);
    assert_eq!(reader.u8(), Ok(0x81));
    assert_eq!(reader.u16(), Ok(0x2345));
    assert_eq!(reader.u32(), Ok(0x6789_abcd));
    assert_eq!(reader.u64(), Ok(0x0123_4567_89ab_cdef));
    assert_eq!(reader.i32(), Ok(-2));
    assert_eq!(reader.i64(), Ok(-3));
    assert_eq!(reader.finish(), Ok(()));
}

#[test]
fn varint_golden_boundaries() {
    let cases: &[(u64, &[u8])] = &[
        (0, &[0]),
        (127, &[127]),
        (128, &[128, 1]),
        (16383, &[255, 127]),
        (16384, &[128, 128, 1]),
        (u64::MAX, &[255, 255, 255, 255, 255, 255, 255, 255, 255, 1]),
    ];
    for &(value, expected) in cases {
        let mut writer = Writer::new();
        writer.uvarint(value);
        assert_eq!(writer.into_bytes(), expected);
        let mut reader = Reader::new(expected, LIMITS);
        assert_eq!(reader.uvarint(), Ok(value));
        assert_eq!(reader.finish(), Ok(()));
    }
    for shift in 0..64 {
        let value = 1_u64 << shift;
        let mut writer = Writer::new();
        writer.uvarint(value);
        let bytes = writer.into_bytes();
        assert_eq!(bytes.len(), (shift + 7) / 7);
        assert_eq!(Reader::new(&bytes, LIMITS).uvarint(), Ok(value));
    }
}

#[test]
fn malformed_varints_preserve_cursor() {
    let cases: &[(&[u8], Error)] = &[
        (&[], Error::Incomplete { needed: 1 }),
        (&[128], Error::Incomplete { needed: 1 }),
        (&[128, 0], Error::NonCanonicalVarint),
        (&[255, 0], Error::NonCanonicalVarint),
        (
            &[128, 128, 128, 128, 128, 128, 128, 128, 128, 0],
            Error::NonCanonicalVarint,
        ),
        (
            &[255, 255, 255, 255, 255, 255, 255, 255, 255, 2],
            Error::Overflow,
        ),
        (&[128; 11], Error::Overflow),
    ];
    for (bytes, error) in cases {
        let mut reader = Reader::new(bytes, LIMITS);
        assert_eq!(reader.uvarint(), Err(error.clone()));
        assert_eq!(reader.remaining(), *bytes);
    }
}

#[test]
fn composite_golden_bytes() {
    let position = Position {
        epoch: 0x0403_0201,
        lsn: 0x0c0b_0a09_0807_0605,
    };
    let version = Version {
        number: 1,
        hash: [0x42; 32],
    };
    let mut writer = Writer::new();
    writer.bytes(&[0xff, 0]);
    writer.text("é").unwrap();
    writer.option::<u16>(None, Writer::u16);
    writer.option(Some(0x1234), Writer::u16);
    writer.list(&[1, 2], |w, v| w.u8(*v));
    writer.position(position);
    writer.version(version);
    let bytes = writer.into_bytes();
    let mut expected = vec![
        2, 255, 0, 2, 0xc3, 0xa9, 0, 1, 0x34, 0x12, 2, 1, 2, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
        1, 0, 0, 0, 0, 0, 0, 0,
    ];
    expected.extend([0x42; 32]);
    assert_eq!(bytes, expected);
    let mut reader = Reader::new(&bytes, LIMITS);
    assert_eq!(reader.bytes(), Ok([255, 0].as_slice()));
    assert_eq!(reader.text(), Ok("é"));
    assert_eq!(reader.option(Reader::u16), Ok(None));
    assert_eq!(reader.option(Reader::u16), Ok(Some(0x1234)));
    assert_eq!(reader.list(Reader::u8), Ok(vec![1, 2]));
    assert_eq!(reader.position(), Ok(position));
    assert_eq!(reader.version(), Ok(version));
    assert_eq!(reader.finish(), Ok(()));
}

#[test]
fn invalid_text_option_and_trailing_bytes() {
    for (bytes, error) in [
        ([1, 0xff, 0, 0].as_slice(), Error::InvalidUtf8),
        ([3, b'e', 0xcc, 0x81].as_slice(), Error::NonNfcText),
    ] {
        let mut reader = Reader::new(bytes, LIMITS);
        assert_eq!(reader.text(), Err(error));
        assert_eq!(reader.remaining(), bytes);
    }
    for tag in 2..=255 {
        let bytes = [tag];
        let mut reader = Reader::new(&bytes, LIMITS);
        assert_eq!(reader.option(Reader::u8), Err(Error::InvalidOption(tag)));
        assert_eq!(reader.remaining(), bytes);
    }
    let mut writer = Writer::new();
    writer.u8(7);
    assert_eq!(writer.text("e\u{301}"), Err(Error::NonNfcText));
    assert_eq!(writer.into_bytes(), [7]);
    assert_eq!(
        Reader::new(&[1], LIMITS).finish(),
        Err(Error::TrailingBytes(1))
    );
}

#[test]
fn bounded_lengths_and_composite_atomicity() {
    let limits = Limits { bytes: 2, items: 1 };
    let mut bytes = Writer::new();
    bytes.uvarint(u64::MAX);
    let bytes = bytes.into_bytes();
    let mut reader = Reader::new(&bytes, limits);
    let expected = if usize::BITS == 64 {
        Error::Limit {
            resource: "bytes",
            actual: u64::MAX,
            max: 2,
        }
    } else {
        Error::Overflow
    };
    assert_eq!(reader.bytes(), Err(expected));
    assert_eq!(reader.remaining(), bytes);
    assert_eq!(
        Reader::new(&[2], limits).list(Reader::u8),
        Err(Error::Limit {
            resource: "items",
            actual: 2,
            max: 1
        })
    );
    assert_eq!(
        Reader::new(&[3, 1, 2, 3], limits).bytes(),
        Err(Error::Limit {
            resource: "bytes",
            actual: 3,
            max: 2
        })
    );
    let mut reader = Reader::new(&[1], limits);
    assert_eq!(
        reader.option(Reader::u64),
        Err(Error::Incomplete { needed: 8 })
    );
    assert_eq!(reader.remaining(), [1]);
    let mut reader = Reader::new(&[1], limits);
    assert_eq!(
        reader.list(Reader::u8),
        Err(Error::Incomplete { needed: 1 })
    );
    assert_eq!(reader.remaining(), [1]);
    let mut reader = Reader::new(&[2, 7], limits);
    assert_eq!(reader.bytes(), Err(Error::Incomplete { needed: 1 }));
    assert_eq!(reader.remaining(), [2, 7]);
    let mut reader = Reader::new(&[0; 11], limits);
    assert_eq!(reader.position(), Err(Error::Incomplete { needed: 1 }));
    assert_eq!(reader.remaining(), [0; 11]);
    let mut reader = Reader::new(&[0; 39], limits);
    assert_eq!(reader.version(), Err(Error::Incomplete { needed: 1 }));
    assert_eq!(reader.remaining(), [0; 39]);
}

#[test]
fn option_lists_require_known_unique_increasing_keys() {
    let bytes = [2, 1, 1, 0x11, 3, 2, 0x33, 0x44];
    let mut reader = Reader::new(&bytes, LIMITS);
    assert_eq!(
        reader.options(&[1, 3]),
        Ok(vec![(1, [0x11].as_slice()), (3, [0x33, 0x44].as_slice())])
    );
    assert_eq!(reader.finish(), Ok(()));
    for (bytes, error) in [
        (
            [2, 1, 0, 1, 0],
            Error::OptionOrder {
                previous: 1,
                actual: 1,
            },
        ),
        (
            [2, 3, 0, 1, 0],
            Error::OptionOrder {
                previous: 3,
                actual: 1,
            },
        ),
        ([2, 1, 0, 2, 0], Error::UnknownOption(2)),
    ] {
        let mut reader = Reader::new(&bytes, LIMITS);
        assert_eq!(reader.options(&[1, 3]), Err(error));
        assert_eq!(reader.remaining(), bytes);
    }
    assert_eq!(Reader::new(&[0], LIMITS).options(&[]), Ok(vec![]));
    let mut reader = Reader::new(&[1, 1, 2, 0], LIMITS);
    assert_eq!(reader.options(&[1]), Err(Error::Incomplete { needed: 1 }));
    assert_eq!(reader.remaining(), [1, 1, 2, 0]);
}
