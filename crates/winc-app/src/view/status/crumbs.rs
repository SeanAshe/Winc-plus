//! 状态栏路径段的收缩：放不下时折叠中间的目录，再截短第一段和文件名。

use super::*;

/// 状态栏上显示的一段路径。
pub(super) struct Shown<'a> {
    /// 在各段中的下标；折叠若干段后的「…」为 `None`。
    pub(super) index: Option<usize>,
    /// 显示的文字。
    pub(super) name: Cow<'a, str>,
}

/// 返回 `shown` 各段连同段间箭头的总宽度（逻辑像素）。
pub(super) fn width(frame: &mut Frame<'_>, shown: &[Shown<'_>], size: f32) -> f32 {
    let names: f32 = (shown.iter())
        .map(|part| frame.measure(FontKind::Ui, size, &part.name))
        .sum();
    names + CRUMB_GAP * shown.len().saturating_sub(1) as f32
}

/// 把各段的名字 `names` 排进 `max`（逻辑像素）宽以内。
///
/// 依次折叠靠近第一段的目录、截短第一段、去掉第一段、截短文件名。只有一段时，`path` 为真
/// 表示它是完整路径，按 [`shorten_path`] 截短。
pub(super) fn fit<'a>(
    frame: &mut Frame<'_>,
    names: &[&'a str],
    path: bool,
    size: f32,
    max: f32,
) -> Vec<Shown<'a>> {
    let whole = |index: usize| Shown {
        index: Some(index),
        name: Cow::Borrowed(names[index]),
    };
    let folded = || Shown {
        index: None,
        name: Cow::Borrowed("…"),
    };
    let Some(last) = names.len().checked_sub(1) else {
        return Vec::new();
    };
    if last == 0 {
        let name = match path {
            true => shorten_path(frame, names[0], size, max),
            false => ellipsize(frame, names[0], size, max),
        };
        return vec![Shown {
            index: Some(0),
            name,
        }];
    }
    // 第 1 到 `hidden` 段折叠成一个「…」；`hidden` 为 0 时全部显示。
    for hidden in 0..last {
        let mut shown = vec![whole(0)];
        shown.extend((hidden > 0).then(folded));
        shown.extend((hidden + 1..=last).map(whole));
        if width(frame, &shown, size) <= max {
            return shown;
        }
    }
    let tail = [folded(), whole(last)];
    let room = max - width(frame, &tail, size) - CRUMB_GAP;
    let [mark, file] = tail;
    if room > MIN_TEXT {
        let root = Shown {
            index: Some(0),
            name: ellipsize(frame, names[0], size, room),
        };
        return vec![root, mark, file];
    }
    let mark_w = frame.measure(FontKind::Ui, size, &mark.name) + CRUMB_GAP;
    let file = Shown {
        index: Some(last),
        name: ellipsize(frame, names[last], size, (max - mark_w).max(0.0)),
    };
    vec![mark, file]
}
