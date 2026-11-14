//! 暂存区内容的计算：上次提交、暂存区、工作区三份文本之间按段、按行的暂存与取消暂存。
//!
//! 差异视图比较的是上次提交与工作区。暂存一段改动时把暂存区与工作区之间的对应改动应用到
//! 暂存区，取消暂存时把上次提交与暂存区之间的对应改动退回，暂存区中其余的内容保持不变。

use super::*;

/// 差异视图中的一行：所在的一侧，以及在该侧文本中的行号（从 0 起）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageLine {
    /// 上次提交一侧的行：暂存它即暂存对该行的删除。
    Old(usize),
    /// 工作区一侧的行：暂存它即暂存该行的加入。
    New(usize),
}

/// 返回上次提交 `head` 到工作区 `work` 的各段改动 `hunks` 在暂存区 `index` 中的状态，下标同
/// `hunks`。
///
/// [`Pick::Apply`] 表示整段已暂存，[`Pick::Skip`] 表示整段未暂存，[`Pick::Lines`] 列出其中
/// 未暂存的行。
pub fn staged_picks(head: &str, index: &str, work: &str, hunks: &[LineHunk]) -> Vec<Pick> {
    let (staged, unstaged) = (line_hunks(head, index), line_hunks(index, work));
    hunks
        .iter()
        .map(|hunk| {
            // 旧行的删除已暂存：该行在暂存区中已不存在。新行已暂存：该行在暂存区中已存在。
            let old: BTreeSet<usize> = (0..hunk.old.len())
                .filter(|k| !covers(&staged, hunk.old.start + k, true))
                .collect();
            let new: BTreeSet<usize> = (0..hunk.new.len())
                .filter(|k| covers(&unstaged, hunk.new.start + k, false))
                .collect();
            if old.is_empty() && new.is_empty() {
                Pick::Apply
            } else if old.len() == hunk.old.len() && new.len() == hunk.new.len() {
                Pick::Skip
            } else {
                Pick::Lines(LineSet { old, new })
            }
        })
        .collect()
}

/// 返回暂存 `hunk` 之后暂存区的内容；`line` 不为 `None` 时只暂存其中这一行。
///
/// `hunk` 是 `head` 到 `work` 的一段改动。已经暂存的部分保持不变。
pub fn stage(
    head: &str,
    index: &str,
    work: &str,
    hunk: &LineHunk,
    line: Option<StageLine>,
) -> String {
    let unstaged = line_hunks(index, work);
    let all = |len: usize| (0..len).collect::<BTreeSet<usize>>();
    let pick = |h: &LineHunk| match line {
        None if touches(&h.new, &hunk.new) => Pick::Apply,
        Some(StageLine::New(at)) if h.new.contains(&at) => {
            let mut new = all(h.new.len());
            new.remove(&(at - h.new.start));
            Pick::Lines(LineSet {
                old: all(h.old.len()),
                new,
            })
        }
        Some(StageLine::Old(at)) => {
            // 该行在暂存区中的位置；已不在暂存区中时无需处理。
            match carried(&line_hunks(head, index), at, true) {
                Some(at) if h.old.contains(&at) => {
                    let mut old = all(h.old.len());
                    old.remove(&(at - h.old.start));
                    Pick::Lines(LineSet {
                        old,
                        new: all(h.new.len()),
                    })
                }
                _ => Pick::Skip,
            }
        }
        _ => Pick::Skip,
    };
    pick_changes(index, work, |_, old| {
        (unstaged.iter().find(|h| h.old == old)).map_or(Pick::Skip, pick)
    })
}

/// 返回取消暂存 `hunk` 之后暂存区的内容；`line` 不为 `None` 时只取消暂存其中这一行。
///
/// `hunk` 是 `head` 到 `work` 的一段改动。未暂存的部分保持不变。
pub fn unstage(
    head: &str,
    index: &str,
    work: &str,
    hunk: &LineHunk,
    line: Option<StageLine>,
) -> String {
    let staged = line_hunks(head, index);
    let one = |at: usize| BTreeSet::from([at]);
    let pick = |h: &LineHunk| match line {
        None if touches(&h.old, &hunk.old) => Pick::Skip,
        Some(StageLine::Old(at)) if h.old.contains(&at) => Pick::Lines(LineSet {
            old: one(at - h.old.start),
            new: BTreeSet::new(),
        }),
        Some(StageLine::New(at)) => {
            // 该行在暂存区中的位置；尚未进入暂存区时无需处理。
            match carried(&line_hunks(index, work), at, false) {
                Some(at) if h.new.contains(&at) => Pick::Lines(LineSet {
                    old: BTreeSet::new(),
                    new: one(at - h.new.start),
                }),
                _ => Pick::Apply,
            }
        }
        _ => Pick::Apply,
    };
    pick_changes(head, index, |_, old| {
        (staged.iter().find(|h| h.old == old)).map_or(Pick::Apply, pick)
    })
}

/// 第 `line` 行是否落在 `hunks` 中某段改动的旧侧（`old` 为真）或新侧之内。
fn covers(hunks: &[LineHunk], line: usize, old: bool) -> bool {
    hunks.iter().any(|h| {
        let range = if old { &h.old } else { &h.new };
        range.contains(&line)
    })
}

/// 两个行区间是否属于同一处改动：都不为空时须有重叠；有一个为空（只是插入位置）时相接即可。
fn touches(a: &Range<usize>, b: &Range<usize>) -> bool {
    if a.is_empty() || b.is_empty() {
        a.start <= b.end && b.start <= a.end
    } else {
        a.start < b.end && b.start < a.end
    }
}

/// 返回一侧文本的第 `line` 行在另一侧文本中的行号；该行在某段改动之内时为 `None`。
///
/// `forward` 为真时从 `hunks` 的旧侧换算到新侧，否则从新侧换算到旧侧。
fn carried(hunks: &[LineHunk], line: usize, forward: bool) -> Option<usize> {
    let mut at = line;
    for h in hunks {
        let (from, to) = if forward {
            (&h.old, &h.new)
        } else {
            (&h.new, &h.old)
        };
        if line < from.start {
            break;
        }
        if line < from.end {
            return None;
        }
        at = at + to.len() - from.len();
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "a\nb\nc\nd\ne\n";
    const WORK: &str = "a\nB\nc\nd\nE\nf\n";

    fn hunks() -> Vec<LineHunk> {
        line_hunks(HEAD, WORK)
    }

    #[test]
    fn nothing_staged_and_everything_staged() {
        let hunks = hunks();
        assert_eq!(hunks.len(), 2);
        assert_eq!(
            staged_picks(HEAD, HEAD, WORK, &hunks),
            [Pick::Skip, Pick::Skip]
        );
        assert_eq!(
            staged_picks(HEAD, WORK, WORK, &hunks),
            [Pick::Apply, Pick::Apply]
        );
    }

    #[test]
    fn staging_one_hunk_leaves_the_other_alone() {
        let hunks = hunks();
        let index = stage(HEAD, HEAD, WORK, &hunks[1], None);
        assert_eq!(index, "a\nb\nc\nd\nE\nf\n");
        assert_eq!(
            staged_picks(HEAD, &index, WORK, &hunks),
            [Pick::Skip, Pick::Apply]
        );
        // 再暂存第一段，暂存区与工作区相同；取消暂存第二段后只剩第一段。
        let both = stage(HEAD, &index, WORK, &hunks[0], None);
        assert_eq!(both, WORK);
        assert_eq!(
            unstage(HEAD, &both, WORK, &hunks[1], None),
            "a\nB\nc\nd\ne\n"
        );
        assert_eq!(unstage(HEAD, &index, WORK, &hunks[1], None), HEAD);
    }

    #[test]
    fn single_lines_are_staged_and_unstaged() {
        let hunks = hunks();
        // 第二段把「e」换成「E」「f」：只暂存「f」的加入。
        let index = stage(HEAD, HEAD, WORK, &hunks[1], Some(StageLine::New(5)));
        assert_eq!(index, "a\nb\nc\nd\ne\nf\n");
        let picks = staged_picks(HEAD, &index, WORK, &hunks);
        let lines = LineSet {
            old: BTreeSet::from([0]),
            new: BTreeSet::from([0]),
        };
        assert_eq!(picks, [Pick::Skip, Pick::Lines(lines)]);
        // 再暂存对「e」的删除。
        let index = stage(HEAD, &index, WORK, &hunks[1], Some(StageLine::Old(4)));
        assert_eq!(index, "a\nb\nc\nd\nf\n");
        // 取消暂存「f」，再取消对「e」的删除，回到上次提交的内容。
        let index = unstage(HEAD, &index, WORK, &hunks[1], Some(StageLine::New(5)));
        assert_eq!(index, "a\nb\nc\nd\n");
        let index = unstage(HEAD, &index, WORK, &hunks[1], Some(StageLine::Old(4)));
        assert_eq!(index, HEAD);
    }

    #[test]
    fn a_change_edited_after_staging_is_partly_staged() {
        // 暂存「B」之后工作区又改成「BB」：暂存区中是第三个版本。
        let work = "a\nBB\nc\nd\ne\n";
        let index = "a\nB\nc\nd\ne\n";
        let hunks = line_hunks(HEAD, work);
        let lines = LineSet {
            old: BTreeSet::new(),
            new: BTreeSet::from([0]),
        };
        assert_eq!(
            staged_picks(HEAD, index, work, &hunks),
            [Pick::Lines(lines)]
        );
        assert_eq!(stage(HEAD, index, work, &hunks[0], None), work);
        assert_eq!(unstage(HEAD, index, work, &hunks[0], None), HEAD);
    }

    #[test]
    fn staged_content_elsewhere_survives_a_toggle() {
        // 暂存区中第一段是第三个版本；暂存第二段不改变第一段在暂存区中的内容。
        let index = "a\nX\nc\nd\ne\n";
        let hunks = hunks();
        assert_eq!(
            stage(HEAD, index, WORK, &hunks[1], None),
            "a\nX\nc\nd\nE\nf\n"
        );
    }

    #[test]
    fn pure_additions_and_deletions() {
        let work = "a\nc\nd\ne\nz\n";
        let hunks = line_hunks(HEAD, work);
        assert_eq!(hunks.len(), 2);
        let index = stage(HEAD, HEAD, work, &hunks[0], None);
        assert_eq!(index, "a\nc\nd\ne\n");
        assert_eq!(
            staged_picks(HEAD, &index, work, &hunks),
            [Pick::Apply, Pick::Skip]
        );
        let index = stage(HEAD, &index, work, &hunks[1], None);
        assert_eq!(index, work);
        assert_eq!(
            unstage(HEAD, &index, work, &hunks[0], None),
            "a\nb\nc\nd\ne\nz\n"
        );
    }
}
