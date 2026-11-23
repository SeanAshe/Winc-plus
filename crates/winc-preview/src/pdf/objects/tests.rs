//! 对象读取的测试：取自文件的宽度、行长和互相指向的引用不引起 panic、溢出或无限递归。

use super::*;

#[test]
fn a_predictor_row_longer_than_the_data_is_refused() {
    assert_eq!(unpredict(&[1, 1, 2], 2, 1), Some(vec![1, 3]));
    assert_eq!(unpredict(&[0, 1, 2], 1 << 60, 1), None);
    assert_eq!(unpredict(&[0, 1, 2], usize::MAX, 1), None);
    assert_eq!(unpredict(&[], 0, 1), None);
}

#[test]
fn cross_reference_fields_wider_than_a_number_are_refused() {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let at = bytes.len();
    bytes.extend_from_slice(
        b"1 0 obj\n<< /Type /XRef /Size 1 /Length 4 \
          /W [9223372036854775807 9223372036854775807 3] >>\nstream\nabcd\nendstream\nendobj\n",
    );
    bytes.extend_from_slice(format!("startxref\n{at}\n%%EOF\n").as_bytes());
    // 读不出交叉引用流，改为逐字节查找对象；两条路径都正常结束。
    let file = File::open(&bytes).unwrap();
    assert!(file.resolve(file.trailer.get(b"Root")).is_none());
}

#[test]
fn a_stream_whose_length_refers_to_itself_is_still_read() {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let at = bytes.len();
    bytes.extend_from_slice(b"5 0 obj\n<< /Length 5 0 R >>\nstream\nabc\nendstream\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for n in 1..6 {
        let line = if n == 5 {
            format!("{at:010} 00000 n \n")
        } else {
            "0000000000 65535 f \n".to_owned()
        };
        bytes.extend_from_slice(line.as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 5 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let file = File::open(&bytes).unwrap();
    // 长度解不出来时按 `endstream` 的位置取数据。
    let stream = file.object(5).unwrap();
    assert_eq!(file.decode(&stream).unwrap(), b"abc\n");
    assert_eq!(file.resolving.get(), 0);
}
