//! 远端搜索和列出文件的测试：条件与结果的编解码、不合格式的数据，以及在本机目录上按条件搜索。

use super::*;

fn request(mode: Mode) -> Request {
    Request {
        query: Query {
            text: "needle".into(),
            case_sensitive: true,
            include: "src, *.rs".into(),
            exclude: "target".into(),
            ..Query::default()
        },
        region: Some("cn".into()),
        limit: 20_000,
        mode,
        root: b"/srv/app".to_vec(),
        paths: vec![b"/srv/app".to_vec(), b"/srv/lib".to_vec()],
        skip: vec![b"/srv/app/src/main.rs".to_vec()],
    }
}

#[test]
fn requests_and_records_survive_encoding() {
    for mode in [
        Mode::Tree { ignored: true },
        Mode::Dependencies,
        Mode::Files,
    ] {
        let sent = request(mode);
        assert_eq!(Request::decode(&sent.encode()).unwrap(), sent);
    }
    let records = [
        Record::File {
            path: "/srv/app/文件.rs".as_bytes().to_vec(),
            matches: vec![LineMatch {
                line: 3,
                cols: 4..10,
                preview: "let needle = 1;".into(),
                highlight: 4..10,
            }],
            lossy: true,
        },
        Record::Skipped(0),
        Record::End {
            truncated: true,
            gave_up: false,
        },
    ];
    let listing = Listing {
        root: b"/srv/app".to_vec(),
        paths: vec![String::new(), "src/main.rs".into()],
        layers: Some(3),
    };
    assert_eq!(Listing::decode(&listing.encode()).unwrap(), listing);
    let descent = Descent {
        path: b"/srv/app/src".to_vec(),
        dirs: 1000,
        entries: 50_000,
    };
    assert_eq!(Descent::decode(&descent.encode()).unwrap(), descent);
    let mut stream = Vec::new();
    for record in &records {
        write_record(&mut stream, record).unwrap();
    }
    let mut input = stream.as_slice();
    for record in &records {
        assert_eq!(read_record(&mut input).unwrap().as_ref(), Some(record));
    }
    assert_eq!(read_record(&mut input).unwrap(), None);
}

#[test]
fn malformed_data_is_rejected() {
    let encoded = request(Mode::Files).encode();
    assert!(Request::decode(&encoded[..encoded.len() - 1]).is_err());
    assert!(Request::decode(&[encoded.as_slice(), b"x"].concat()).is_err());

    let mut stream = Vec::new();
    write_record(&mut stream, &Record::Skipped(7)).unwrap();
    // 在一条结果中间结束、长度超过上限、未知的种类。
    let cut = &stream[..stream.len() - 1];
    assert!(read_record(&mut &cut[..]).is_err());
    let huge = u32::MAX.to_le_bytes();
    assert!(read_record(&mut &huge[..]).is_err());
    let unknown = [1, 0, 0, 0, 9];
    assert!(read_record(&mut &unknown[..]).is_err());
}

/// 在本机目录上运行远端那一侧：结果逐条写出，「包含」之外的不搜，有未保存修改的文件只报告不读，
/// 最后是结束的一条。
#[cfg(unix)]
#[test]
fn serving_searches_the_tree_and_reports_skipped_files() {
    use std::os::unix::ffi::OsStrExt;

    let dir = std::env::temp_dir().join(format!("winc-search-host-{}", std::process::id()));
    let write = |name: &str, text: &str| {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write("src/main.rs", "needle\n");
    write("src/lib.rs", "a needle\nneedle\n");
    write("notes.md", "needle\n");
    let bytes = |name: &str| dir.join(name).as_os_str().as_bytes().to_vec();
    let request = Request {
        query: Query {
            text: "needle".into(),
            include: "*.rs".into(),
            ..Query::default()
        },
        region: None,
        limit: 100,
        mode: Mode::Tree { ignored: false },
        root: bytes(""),
        paths: vec![bytes("")],
        skip: vec![bytes("src/main.rs")],
    };
    let mut output = Vec::new();
    search(&request.encode(), &mut output).unwrap();

    let mut input = output.as_slice();
    let mut records = Vec::new();
    while let Some(record) = read_record(&mut input).unwrap() {
        records.push(record);
    }
    let end = records.pop().unwrap();
    assert_eq!(
        end,
        Record::End {
            truncated: false,
            gave_up: false
        }
    );
    records.sort_by_key(|record| matches!(record, Record::File { .. }));
    assert_eq!(records[0], Record::Skipped(0));
    let Record::File { path, matches, .. } = &records[1] else {
        panic!("{records:?}");
    };
    assert_eq!(path, &bytes("src/lib.rs"));
    assert_eq!(matches.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}
