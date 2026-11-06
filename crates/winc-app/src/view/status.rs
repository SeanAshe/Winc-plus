//! 状态栏绘制。
//!
//! 左侧是当前文档的路径和临时消息；右侧依次是光标位置、行尾、编码、缩进和语言。行尾、编码、
//! 缩进可点击，点击后弹出菜单修改；语言可点击，点击后列出所有语言。

use winc_core::Lines;
use winc_i18n::t;

use super::*;
use crate::app::{StatusEntry, StatusItem, StatusKey, StatusMotion, indent_label};
use crate::icons::Direction;

/// 路径中相邻两段文字之间的距离（逻辑像素），箭头绘制在正中。
const CRUMB_GAP: f32 = 22.0;

/// 状态栏右侧相邻两项之间的距离（逻辑像素）。
const STATUS_GAP: f32 = 15.0;

/// 路径中一段的底色在文字两侧各超出的宽度（逻辑像素）。
///
/// 箭头半宽约 2.4 逻辑像素（`6.5 / 4 + 1.5 / 2`：箭头尺寸 6.5 的四分之一加半个笔宽），因此
/// 底色与相邻箭头之间还留有约 3.6 逻辑像素的间隙（`CRUMB_GAP / 2 - CRUMB_PAD - 2.4`）。
const CRUMB_PAD: f32 = 5.0;

/// 文档组的项，按从左到右的顺序。
const DOCUMENT: &[StatusKey] = &[
    StatusKey::Position,
    StatusKey::LineEnding,
    StatusKey::Encoding,
    StatusKey::Indent,
    StatusKey::Language,
];

/// 图片预览组的项，按从左到右的顺序。
const PICTURE: &[StatusKey] = &[
    StatusKey::PictureMissing,
    StatusKey::PictureSize,
    StatusKey::PictureFile,
    StatusKey::PictureFormat,
];

/// 分支图标的宽度（逻辑像素），位于分支名左侧。
const BRANCH_ICON: f32 = 14.0;

/// 下拉箭头的宽度（逻辑像素），位于分支名右侧。
const BRANCH_ARROW: f32 = 12.0;

/// 分支图标的线条超出内容左边的最大距离（逻辑像素，经验值），裁剪时为其留出余量。
const ICON_OVERHANG: f32 = 2.0;

/// 从 `right` 起向左绘制状态栏右侧的一组项，并把可点击项的命中区域追加到 `hits`。
///
/// `group` 列出该组可能出现的全部项，`entries` 是本帧要显示的项，宽度与不透明度由 `motion`
/// 计算。`opacity` 为整组内容的不透明度，小于 1 时各项随之淡化且不可点击。返回时 `right` 位于
/// 最左一项左侧一个 `STATUS_GAP` 处。
#[allow(clippy::too_many_arguments)]
fn draw_group(
    frame: &mut Frame<'_>,
    app: &App,
    motion: &mut StatusMotion,
    animate: bool,
    group: &[StatusKey],
    entries: Vec<(StatusKey, String, Option<StatusItem>)>,
    opacity: f32,
    bar: Rect,
    right: &mut f32,
    hits: &mut Vec<(Rect, StatusItem)>,
) {
    let size = app.fonts.ui_small;
    // 占用宽度按最宽的数字计算每一位数字：光标移动时位数不变，宽度也不变。
    let widest = ('0'..='9')
        .map(|digit| {
            (
                frame.measure(FontKind::Ui, size, digit.encode_utf8(&mut [0; 4])),
                digit,
            )
        })
        .fold((0.0, '0'), |a, b| if b.0 > a.0 { b } else { a })
        .1;
    let entries = entries
        .into_iter()
        .map(|(key, text, item)| {
            let extra = if item == Some(StatusItem::Branch) {
                BRANCH_ICON + BRANCH_ARROW
            } else {
                0.0
            };
            let stable: String = text
                .chars()
                .map(|c| if c.is_ascii_digit() { widest } else { c })
                .collect();
            StatusEntry {
                key,
                content: frame.measure(FontKind::Ui, size, &text) + extra,
                width: frame.measure(FontKind::Ui, size, &stable) + extra,
                text,
                item,
            }
        })
        .collect();

    let p = app.palette;
    for row in motion.sync(group, entries, animate).into_iter().rev() {
        let w = row.width;
        *right -= w;
        // 可点击项与路径各段的样式相同：悬停时铺按钮底色，菜单打开时显示为按下状态。
        if let Some(item) = row.item.filter(|_| opacity >= 1.0) {
            let pill = Rect::new(
                *right - CRUMB_PAD,
                bar.y + 4.0,
                w + 2.0 * CRUMB_PAD,
                bar.h - 8.0,
            );
            draw_button(
                frame,
                p,
                pill,
                bar,
                app.hover == Hover::Status(item),
                app.menu_button == Some(MenuButton::Status(item)),
            );
            hits.push((pill, item));
        }
        // 内容靠右对齐。宽度过渡途中内容可能宽于占用宽度，超出部分不绘制，以免与左侧的项重叠。
        // 裁剪范围向左多留 `ICON_OVERHANG`：分支图标的线条略超出内容的左边，不留时会被裁去。
        let clip = bar.intersect(Rect::new(
            *right - ICON_OVERHANG,
            bar.y,
            w + ICON_OVERHANG,
            bar.h,
        ));
        if let Some((text, content, alpha)) = &row.previous {
            let x = *right + w - content;
            draw_content(frame, app, x, bar, clip, text, row.key, alpha * opacity);
        }
        let (text, content) = &row.text;
        let x = *right + w - content;
        draw_content(frame, app, x, bar, clip, text, row.key, row.alpha * opacity);
        *right -= STATUS_GAP * row.share;
    }
}

/// 在 `x` 处绘制 `key` 一项的内容，不透明度为 `alpha`，超出 `clip` 的部分不绘制。
///
/// 分支名的左侧是分支图标，右侧是下拉箭头。新版本提示使用强调色。
#[allow(clippy::too_many_arguments)]
fn draw_content(
    frame: &mut Frame<'_>,
    app: &App,
    x: f32,
    bar: Rect,
    clip: Rect,
    text: &str,
    key: StatusKey,
    alpha: f32,
) {
    if alpha <= 0.0 {
        return;
    }
    let p = app.palette;
    let fade = |c: Color| c.alpha(c.a * alpha);
    let ink = if key == StatusKey::Update {
        p.accent
    } else {
        p.text_muted
    };
    let branch = key == StatusKey::Branch;
    let icon = if branch { BRANCH_ICON } else { 0.0 };
    if branch {
        let cy = icon_center_y(app, bar, app.fonts.ui_small);
        let text_w = frame.measure(FontKind::Ui, app.fonts.ui_small, text);
        icons::branch_small(frame, (x + 4.0, cy), fade(p.text_muted), clip);
        let arrow = (x + icon + text_w + 7.0, cy);
        icons::chevron(frame, arrow, 5.5, Direction::Down, fade(p.text_dim), clip);
    }
    ui_text_small(frame, app, x + icon, bar, text, fade(ink), clip);
}

/// 绘制状态栏，并更新路径段和右侧可点击项的命中区域（`crumb_hits`、`status_hits`）。
pub(super) fn draw_status(app: &mut App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let bar = app.layout.status;
    frame.rect(bar, p.frame_bg);
    // 隐藏时只剩窗口底边的边距，没有可以点击的项。
    if !app.status_bar_shown() {
        app.status_hits.clear();
        app.crumb_hits.clear();
        return;
    }
    // 绘制各组时同时读取 `app`，因此过渡状态先移出，绘制结束后恢复。
    let animate = app.animate_status();
    let mut motion = std::mem::take(&mut app.status_motion);

    let doc = app.editor.active();
    // 无法显示文本的文件没有光标、编码等信息；仍在读取中的文件这些信息也尚不可用。
    let shown = doc.filter(|d| d.shows_text() && !d.is_pending());
    let mut right = bar.right() - 14.0;
    let mut status_hits = Vec::new();

    // 最右边始终是设置按钮，不随模块和当前文档变化：悬停时与其他可点击项一样铺按钮底色。
    let item = StatusItem::Settings;
    let slot = right - icons::SIDEBAR_WIDTH;
    right -= icons::SIDEBAR_WIDTH + STATUS_GAP;
    let pill = Rect::new(
        slot - CRUMB_PAD,
        bar.y + 4.0,
        icons::SIDEBAR_WIDTH + 2.0 * CRUMB_PAD,
        bar.h - 8.0,
    );
    let hovered = app.hover == Hover::Status(item);
    draw_button(frame, p, pill, bar, hovered, false);
    let center = (slot + icons::SIDEBAR_WIDTH / 2.0, bar.y + bar.h / 2.0);
    let color = if hovered { p.text } else { p.text_muted };
    icons::gear(frame, center, color, bar);
    status_hits.push((pill, item));

    // 终端模块开着时左边是终端面板的开关：悬停时与其他可点击项一样铺按钮底色，面板开着时
    // 图标亮、收着时灰。面板收着、后台有命令结束了时右上角一个小点（失败了是红的），和标签上的
    // 一样。
    // 开关模块时淡入淡出，左边的几项跟着滑过去（`App::track_modules`）；消失途中点不到。
    let enabled = app.terminal_enabled();
    let share = app.module_fx.terminal_icon.share(enabled);
    let opacity = app.module_fx.terminal_icon.opacity(enabled);
    if share > 0.0 {
        let item = StatusItem::Terminal;
        let slot = right - icons::SIDEBAR_WIDTH;
        right -= (icons::SIDEBAR_WIDTH + STATUS_GAP) * share;
        let pill = Rect::new(
            slot - CRUMB_PAD,
            bar.y + 4.0,
            icons::SIDEBAR_WIDTH + 2.0 * CRUMB_PAD,
            bar.h - 8.0,
        );
        let open = app.terminal.open;
        let hovered = enabled && app.hover == Hover::Status(item);
        frame.with_opacity(opacity, |frame| {
            draw_button(frame, p, pill, bar, hovered, false);
        });
        let center = (slot + icons::SIDEBAR_WIDTH / 2.0, bar.y + bar.h / 2.0);
        let color = if open || hovered {
            p.text
        } else {
            p.text_muted
        };
        icons::terminal(frame, center, color.alpha(opacity), bar);
        let unseen = app.terminal.tabs.iter().filter_map(|t| t.unseen);
        if !open && let Some(failed) = unseen.reduce(|a, b| a || b) {
            let dot = if failed { p.error } else { p.accent };
            let at = Rect::new(pill.right() - 7.0, pill.y + 2.0, 5.0, 5.0);
            frame.rect_clipped(at, dot.alpha(opacity), 2.5, bar);
        }
        if app.terminal_enabled() {
            status_hits.push((pill, item));
        }
    }

    // 后台检查更新找到了新版本：在终端开关左边提示，点了打开更新窗口（macOS），或者下载安装、
    // 装好以后重新启动（其他平台）。
    let update = app.shared.update.borrow().clone();
    let mut items = Vec::new();
    if let Some(version) = update {
        use crate::winc::UpdateState;
        let label = match &*app.shared.update_state.borrow() {
            UpdateState::Downloading(_) => t!("workspace-status-update-downloading").to_owned(),
            UpdateState::Ready(_) => t!("workspace-status-update-ready").to_owned(),
            _ => t!("workspace-status-update", version = version),
        };
        items.push((StatusKey::Update, label, Some(StatusItem::Update)));
    }
    draw_group(
        frame,
        app,
        &mut motion,
        animate,
        &[StatusKey::Update],
        items,
        1.0,
        bar,
        &mut right,
        &mut status_hits,
    );

    // 图片、SVG 预览：依次显示图片大小（像素）、文件大小、格式，以及 SVG 中未能绘制的特性。
    // 切换源代码与预览的按钮在标签栏右端（`Layout::preview`）。
    let mut items = Vec::new();
    if let Some(picture) = doc.and_then(|d| d.picture()) {
        if !picture.missing().is_empty() {
            let features = picture.missing().join(t!("workspace-list-separator"));
            let text = t!("workspace-status-not-shown", features = features);
            items.push((StatusKey::PictureMissing, text, None));
        }
        let size = format!("{} × {}", picture.width, picture.height);
        items.push((StatusKey::PictureSize, size, None));
        let file = winc_preview::image::size_label(picture.file_size);
        items.push((StatusKey::PictureFile, file, None));
        items.push((StatusKey::PictureFormat, picture.format.to_owned(), None));
    }
    draw_group(
        frame,
        app,
        &mut motion,
        animate,
        PICTURE,
        items,
        1.0,
        bar,
        &mut right,
        &mut status_hits,
    );

    let mut items = Vec::new();
    if let Some(doc) = shown {
        let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
        let selected = doc.selection.to() - doc.selection.from();
        let language = doc.highlighter.language();
        let extra = doc.extra_selections();
        let position = if extra.is_empty() {
            match selected {
                0 => format!("{}:{}", line + 1, col + 1),
                n => t!(
                    "workspace-status-position-selected",
                    position = format!("{}:{}", line + 1, col + 1),
                    count = n
                ),
            }
        } else {
            // 多光标：显示选区个数和一共选中的字符数。
            let count = extra.len() + 1;
            match selected + doc.extra_selected_chars() {
                0 => t!("workspace-status-selections", count = count),
                n => t!(
                    "workspace-status-selections-chars",
                    count = count,
                    chars = n
                ),
            }
        };
        let indent = indent_label(doc.indent(), doc.tab_width(app.editor.tab_width));
        // 仅供查看的标签（格式化输出）没有可修改的缩进、行尾和编码。
        let clickable = |item| doc.title.is_none().then_some(item);

        items = vec![
            (StatusKey::Position, position, None),
            (
                StatusKey::LineEnding,
                doc.line_ending.label().to_owned(),
                clickable(StatusItem::LineEnding),
            ),
            (
                StatusKey::Encoding,
                doc.encoding.label().to_owned(),
                clickable(StatusItem::Encoding),
            ),
            (StatusKey::Indent, indent, clickable(StatusItem::Indent)),
            (
                StatusKey::Language,
                language.name.to_owned(),
                clickable(StatusItem::Language),
            ),
        ];
    }
    draw_group(
        frame,
        app,
        &mut motion,
        animate,
        DOCUMENT,
        items,
        1.0,
        bar,
        &mut right,
        &mut status_hits,
    );

    if let Some(doc) = shown {
        // 只读和大文件是需要留意的状态，用带底色的小胶囊排在右侧一组的最左边。
        let flags = [
            (doc.readonly, t!("workspace-status-readonly"), true),
            (doc.is_large(), t!("workspace-status-large-file"), false),
        ];

        for (_, text, locked) in flags.into_iter().filter(|(on, ..)| *on) {
            let icon_w = if locked { 13.0 } else { 0.0 };
            let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text) + icon_w + 16.0;
            let chip = Rect::new(right + 8.0 - w, bar.y + 4.0, w, bar.h - 8.0);
            frame.rounded(chip, p.selected_inactive, chip.h / 2.0);

            if locked {
                let cy = icon_center_y(app, chip, app.fonts.ui_small);
                icons::lock(frame, (chip.x + 13.0, cy), p.text_muted, bar);
            }

            let x = chip.x + 8.0 + icon_w;
            ui_text_small(frame, app, x, chip, text, p.text_muted, bar);
            right = chip.x - 10.0;
        }
    }

    // Git 模块开着、目录在仓库里时，右侧一组的最左边是分支名，有上游时跟着领先落后的提交数，
    // 有分支操作在跑时跟着在做什么。点了弹分支菜单。
    // 开关模块、换仓库时淡入淡出（`App::track_modules`），消失途中用最后绘制的分支名；左边的
    // 路径能占的宽度跟着慢慢变。
    let live = app
        .git
        .status
        .as_ref()
        .and_then(branch_label)
        .map(|mut label| {
            if let Some(running) = &app.branch.running {
                label.push_str(" · ");
                label.push_str(&t!(
                    "workspace-status-branch-running",
                    task = running.as_str()
                ));
            } else if let Some(operation) = app.git.status.as_ref().and_then(|s| s.operation) {
                label.push_str(" · ");
                let operation = operation.label();
                label.push_str(&t!(
                    "workspace-status-branch-operation",
                    operation = operation
                ));
            }
            label
        });
    let share = app.module_fx.branch.share(live.is_some());
    let opacity = app.module_fx.branch.opacity(live.is_some());
    if live.is_some() {
        app.module_fx.branch_label.clone_from(&live);
    }
    if let Some(label) = live.or_else(|| app.module_fx.branch_label.clone())
        && share > 0.0
    {
        let start = right;
        let items = vec![(StatusKey::Branch, label, Some(StatusItem::Branch))];
        draw_group(
            frame,
            app,
            &mut motion,
            animate,
            &[StatusKey::Branch],
            items,
            opacity,
            bar,
            &mut right,
            &mut status_hits,
        );
        right = start - (start - right) * share;
    }

    let left = Rect::new(bar.x, bar.y, (right - 16.0 - bar.x).max(0.0), bar.h);
    let mut x = bar.x + 14.0;
    let mut hits = Vec::new();

    if doc.is_some() {
        let crumbs = app.crumbs();
        // 工作区以外的文件只有一段完整路径，最多占左侧的一半，放不下时从中间截短并保留完整
        // 文件名：长路径能占满大半个状态栏，把临时消息挤掉。
        let half = left.w / 2.0;

        for (i, crumb) in crumbs.iter().enumerate() {
            let name = if crumbs.len() == 1 && crumb.path.is_some() {
                shorten_path(frame, &crumb.name, app.fonts.ui_small, half)
            } else {
                Cow::Borrowed(crumb.name.as_str())
            };
            if i > 0 {
                let arrow = (
                    x + CRUMB_GAP / 2.0,
                    icon_center_y(app, bar, app.fonts.ui_small),
                );
                icons::chevron(frame, arrow, 6.5, Direction::Right, p.text_dim, left);
                x += CRUMB_GAP;
            }

            // 有路径的段可点击：悬停时铺按钮底色，其目录菜单打开时显示为按下状态。
            if crumb.path.is_some() {
                let w = frame.measure(FontKind::Ui, app.fonts.ui_small, &name);
                let pill = Rect::new(x - CRUMB_PAD, bar.y + 4.0, w + 2.0 * CRUMB_PAD, bar.h - 8.0);
                let hovered = app.hover == Hover::Crumb(i);
                draw_button(
                    frame,
                    p,
                    pill,
                    left,
                    hovered,
                    app.menu_button == Some(MenuButton::Crumb(i)),
                );
                if pill.x < left.right() {
                    let visible = pill.w.min(left.right() - pill.x);
                    hits.push((Rect::new(pill.x, pill.y, visible, pill.h), i));
                }
            }

            let color = if i + 1 == crumbs.len() {
                p.text
            } else {
                p.text_muted
            };
            x = ui_text_small(frame, app, x, bar, &name, color, left);
        }

        // 配置文件中光标所在的键路径：接在文件名之后，颜色较淡，放不下时从中间截短。
        if let Some(path) = app.key_path() {
            let arrow = (
                x + CRUMB_GAP / 2.0,
                icon_center_y(app, bar, app.fonts.ui_small),
            );
            icons::chevron(frame, arrow, 6.5, Direction::Right, p.text_dim, left);
            x += CRUMB_GAP;
            let room = (left.right() - x).max(0.0);
            let path = ellipsize(frame, &path, app.fonts.ui_small, room);
            x = ui_text_small(frame, app, x, bar, &path, p.text_muted, left);
        }

        x += 24.0;
    }

    // 消息放不下时从中间截短；可用宽度不超过 60 逻辑像素时不显示。
    let room = left.right() - x;

    let progress = app.format_in_progress().map(|task| match task {
        winc_editor::format::Task::Format => t!("workspace-status-formatting"),
        winc_editor::format::Task::Fix => t!("workspace-status-fixing"),
    });
    if let Some(message) = app.status_message.as_deref().or(progress)
        && room > 60.0
    {
        let message = ellipsize(frame, message, app.fonts.ui_small, room);
        ui_text_small(frame, app, x, bar, &message, p.status_message, left);
    }

    app.status_motion = motion;
    app.crumb_hits = hits;
    app.status_hits = status_hits;
}

/// 状态栏上的分支：分支名（分离头指针时是提交号），领先、落后时加上 ↑、↓ 和提交数。
fn branch_label(status: &winc_editor::git::Status) -> Option<String> {
    use winc_editor::git::Head;
    let mut label = match status.head.as_ref()? {
        Head::Branch(name) => name.clone(),
        Head::Detached(oid) => format!("({oid})"),
    };
    if status.ahead > 0 {
        label.push_str(&format!(" ↑{}", status.ahead));
    }
    if status.behind > 0 {
        label.push_str(&format!(" ↓{}", status.behind));
    }
    Some(label)
}
