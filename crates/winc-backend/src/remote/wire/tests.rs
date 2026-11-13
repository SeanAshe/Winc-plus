//! 帧和编解码：往返一致，以及对不合约定的输入的拒绝。

use super::*;

#[test]
fn frames_survive_a_round_trip_and_stop_at_a_clean_end() {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, kind::READ, 7, b"payload").unwrap();
    write_frame(&mut bytes, kind::OK, u32::MAX, b"").unwrap();
    let mut input = bytes.as_slice();
    assert_eq!(
        read_frame(&mut input).unwrap(),
        Some(Frame {
            kind: kind::READ,
            id: 7,
            payload: b"payload".to_vec()
        })
    );
    assert_eq!(
        read_frame(&mut input).unwrap(),
        Some(Frame {
            kind: kind::OK,
            id: u32::MAX,
            payload: Vec::new()
        })
    );
    assert_eq!(read_frame(&mut input).unwrap(), None);
}

#[test]
fn malformed_frames_are_rejected() {
    // 长度小于头部、超过上限：协议错误，不按该长度分配内存。
    for len in [0u32, 4, MAX_FRAME as u32 + 1, u32::MAX] {
        let mut bytes = len.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0; 16]);
        let error = read_frame(&mut bytes.as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{len}");
    }
    // 帧未读完对方就关闭。
    let mut bytes = Vec::new();
    write_frame(&mut bytes, kind::OK, 1, b"abcdef").unwrap();
    for cut in [1, 3, 6, bytes.len() - 1] {
        let error = read_frame(&mut &bytes[..cut]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof, "{cut}");
    }
    // 载荷超过上限时不写出。
    let mut out = Vec::new();
    let error = write_frame(&mut out, kind::OK, 1, &vec![0; MAX_FRAME]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(out.is_empty());
    write_frame(&mut out, kind::OK, 1, &vec![0; MAX_READ as usize + 16]).unwrap();
}

#[test]
fn fields_are_read_back_in_the_order_they_were_written() {
    let payload = Writer::default()
        .u8(9)
        .u32(0xDEAD_BEEF)
        .u64(u64::MAX - 1)
        .bytes(b"/tmp/\xFFname")
        .bytes(b"")
        .finish();
    let mut input = Reader::new(&payload);
    assert_eq!(input.u8().unwrap(), 9);
    assert_eq!(input.u32().unwrap(), 0xDEAD_BEEF);
    assert_eq!(input.u64().unwrap(), u64::MAX - 1);
    assert_eq!(input.bytes().unwrap(), b"/tmp/\xFFname");
    assert_eq!(input.bytes().unwrap(), b"");
    input.finish().unwrap();

    // 字段不完整、字节串的长度超出载荷、读完后还有剩余：都是协议错误。
    assert!(Reader::new(&payload[..3]).u32().is_err());
    assert!(Reader::new(&[5, 0, 0, 0, b'a']).bytes().is_err());
    let mut input = Reader::new(&payload);
    input.u8().unwrap();
    assert!(input.finish().is_err());
}

#[test]
fn metadata_and_errors_survive_a_round_trip() {
    let full = Metadata {
        kind: Kind::Symlink,
        len: 1 << 40,
        modified: Some(Stamp {
            secs: -5,
            nanos: 999_999_999,
        }),
        readonly: true,
        unix: Some(Unix {
            mode: 0o100_644,
            uid: 1000,
            nlink: 2,
            dev: u64::MAX,
            ino: 42,
        }),
    };
    let bare = Metadata {
        kind: Kind::Dir,
        len: 0,
        modified: None,
        readonly: false,
        unix: None,
    };
    for metadata in [full, bare] {
        let payload = write_metadata(Writer::default(), &metadata).finish();
        let mut input = Reader::new(&payload);
        assert_eq!(read_metadata(&mut input).unwrap(), metadata);
        input.finish().unwrap();
    }
    for kind in [Kind::File, Kind::Dir, Kind::Symlink, Kind::Other] {
        assert_eq!(read_kind(kind_byte(kind)).unwrap(), kind);
    }
    assert!(read_kind(4).is_err());

    // 错误的种类和说明原样传递；不在表中的种类按「其他」传递；说明过长时在字符边界截断。
    let error = decode_error(&encode_error(&io::Error::new(
        io::ErrorKind::NotFound,
        "没有这个文件",
    )));
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(error.to_string(), "没有这个文件");
    let error = decode_error(&encode_error(&io::ErrorKind::TimedOut.into()));
    assert_eq!(error.kind(), io::ErrorKind::Other);
    let long = "字".repeat(400);
    let error = decode_error(&encode_error(&io::Error::other(long)));
    assert_eq!(error.to_string(), "字".repeat(170));
    assert_eq!(decode_error(&[]).kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        decode_error(&[200, 0, 0, 0, 0]).kind(),
        io::ErrorKind::Other
    );
}
