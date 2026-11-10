//! `memo` 的测试：旧条目的清理时机、使用中条目的保留、读取记录的排序与去重。

use super::*;

#[test]
fn sweep_drops_entries_no_block_uses() {
    let mut memo = Memo::default();
    for k in 0..(SLACK as u64 + 10) {
        memo.measures.insert(k, (0.0, 0.0));
    }
    // 旧条目未达到上限时不清理。
    let many: Vec<u64> = (0..10).collect();
    memo.sweep([&many[..]].into_iter());
    assert_eq!(memo.len(), SLACK + 10);
    // 超过上限时只保留使用中的条目。
    memo.sweep([&[1, 2][..], &[2, 5][..]].into_iter());
    assert_eq!(memo.len(), 3);
    assert!(memo.measures.contains_key(&5));
    // 读取过的条目按块记录并去重。
    memo.touch(7);
    memo.touch(3);
    memo.touch(7);
    assert_eq!(&*memo.take_touched(), &[3, 7]);
    assert!(memo.take_touched().is_empty());
}
