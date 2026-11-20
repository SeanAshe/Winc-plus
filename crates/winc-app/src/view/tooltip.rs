//! 悬停提示：停留满 `TOOLTIP_DELAY` 后淡入，按目标放在按钮下方、旁边或从鼠标处开始。

use super::*;

/// 绘制按钮的悬停提示。
///
/// 鼠标在按钮上停留满 `TOOLTIP_DELAY`（500 毫秒）才显示，避免鼠标扫过时闪现。提示位于按钮
/// 下方并水平居中；下方放不下时（状态栏上的按钮）改在上方；概览标尺上的提示位于标尺左侧。
pub(super) fn draw_tooltip(app: &App, frame: &mut Frame<'_>) {
    // 鼠标刚离开的提示在原处淡出。
    if let Some((button, label, place, clock)) = &app.overlays.tooltip.leaving {
        draw_tip(app, frame, (*button, label, *place), clock.fade_out());
    }
    // 停留时间不足时直接返回，不拼接提示文字，也不查找命中框。
    let rested = app
        .overlays
        .tooltip
        .since
        .is_some_and(|since| since.elapsed() >= TOOLTIP_DELAY);
    if !rested {
        return;
    }
    let (button, label): (Rect, String) = match workspace::panel::tooltip(app) {
        Some(tip) => tip,
        None => match app.pointer.hover {
            Hover::Search(button) => (
                SearchParts::new(app.layout.search).rect(button),
                button.label(),
            ),
            Hover::TreeTool(tool) => match app.hits.tree_tools.iter().find(|(_, t)| *t == tool) {
                Some(&(button, _)) => (button, tool.label().to_owned()),
                None => return,
            },
            Hover::TreeMore => match app.hits.tree_more {
                Some(button) => (button, t!("workspace-tip-more-actions").to_owned()),
                None => return,
            },
            Hover::SidebarButton => (
                app.layout.sidebar_button,
                crate::layout::with_key(t!("cmd-toggle-sidebar"), Some(Action::ToggleSidebar)),
            ),
            Hover::Browser => (app.layout.browser, t!("preview-browser-open").to_owned()),
            Hover::Preview(mode) => {
                use winc_editor::PreviewMode::*;
                let [source, split, preview] = crate::layout::preview_segments(app.layout.preview);
                // 「预览」对应 ⇧⌘V（在源代码与预览之间切换）。
                match mode {
                    Source => (source, t!("preview-mode-source").to_owned()),
                    Split => (split, t!("preview-mode-split").to_owned()),
                    Preview => (
                        preview,
                        crate::layout::with_key(
                            t!("preview-mode-preview"),
                            Some(Action::TogglePreview),
                        ),
                    ),
                }
            }
            // 格式工具栏的按钮：名字和快捷键。
            Hover::SelectionTool(tool) => {
                let Some(&(rect, _)) = app.hits.selection_tools.iter().find(|(_, t)| *t == tool)
                else {
                    return;
                };
                (
                    rect,
                    crate::layout::with_key(tool.label(), Some(tool.action())),
                )
            }
            // Markdown 预览中的链接：提示地址，脚注的编号提示脚注的内容；可编辑的预览中另起一行
            // 说明怎样打开。
            Hover::MarkdownLink(i) | Hover::MarkdownLinkText(i)
                if !app.markdown_link_is_control(i) =>
            {
                let Some((rect, url)) = app.hits.markdown_links.get(i) else {
                    return;
                };
                let note = (url.strip_prefix("#fn-"))
                    .and_then(|n| app.preview.markdown.parsed.as_ref()?.footnote_text(n));
                let text = match note {
                    Some(note) => note,
                    None if url.starts_with("#fnref-") => return,
                    None => url.clone(),
                };
                // 无法打开的链接另起一行显示原因。
                let text = match app.preview.markdown.links.get(url) {
                    Some(broken) => format!("{text}\n{}", broken.reason()),
                    None => text,
                };
                let key = if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl"
                };
                match app.pointer.hover {
                    Hover::MarkdownLinkText(_) => {
                        let hint = t!("preview-markdown-link-hint", key = key);
                        (*rect, format!("{text}\n{hint}"))
                    }
                    _ => (*rect, text),
                }
            }
            // 日志列表中的作者头像：提示作者名和邮箱。
            Hover::Log(crate::app::git::log::LogTarget::Avatar(i)) => {
                let hit = app
                    .hits
                    .log
                    .iter()
                    .find(|(_, t)| *t == crate::app::git::log::LogTarget::Avatar(i));
                let commit = app
                    .editor
                    .active()
                    .and_then(|d| d.log.as_deref())
                    .and_then(|v| v.commits.get(i));
                match (hit, commit) {
                    (Some(&(rect, _)), Some(c)) => (rect, format!("{} <{}>", c.author, c.email)),
                    _ => return,
                }
            }
            // 提交图中线段的目标：点击后跳转到的提交。
            Hover::Log(crate::app::git::log::LogTarget::Line(row, line)) => {
                let target = crate::app::git::log::LogTarget::Line(row, line);
                if !app.hits.log.iter().any(|(_, t)| *t == target) {
                    return;
                }
                // 线段的命中框很小，提示放在鼠标下方，不按命中框居中。
                let (mx, my) = app.mouse();
                let rect = Rect::new(mx, my + 6.0, 0.0, 0.0);
                let commit = app.log_line_target(row, line).and_then(|j| {
                    app.editor
                        .active()
                        .and_then(|d| d.log.as_deref())
                        .and_then(|v| v.commits.get(j))
                });
                let label = match commit {
                    Some(c) => t!(
                        "workspace-tip-log-go-to",
                        commit = crate::app::git::log::short(&c.oid),
                        subject = c.subject.as_str()
                    ),
                    None => t!("workspace-tip-log-target-not-loaded").to_owned(),
                };
                (rect, label)
            }
            // 日志分支面板中截短了的分支、标签和目录名：提示全名。
            Hover::Log(crate::app::git::log::LogTarget::BranchRow(i))
                if app.git.log.long_branches.contains(&i) =>
            {
                use crate::app::git::log::{BranchRow, LogTarget};
                let hit = app
                    .hits
                    .log
                    .iter()
                    .find(|(_, t)| *t == LogTarget::BranchRow(i));
                let row = app.log_branch_rows().into_iter().nth(i);
                let name = match row {
                    Some(BranchRow::Branch { index, .. }) => {
                        app.git.branches.get(index).map(|b| b.name.clone())
                    }
                    Some(BranchRow::Tag(index)) => app.git.tags.get(index).cloned(),
                    Some(BranchRow::Folder { path, .. }) => Some(path),
                    _ => None,
                };
                match (hit, name) {
                    (Some(&(rect, _)), Some(name)) => (rect, name),
                    _ => return,
                }
            }
            // 日志详情中目录被截短或文件被改名的条目：显示完整路径。
            Hover::Log(crate::app::git::log::LogTarget::File(i))
                if app.git.log.long_files.contains(&i) =>
            {
                let hit = app
                    .hits
                    .log
                    .iter()
                    .find(|(_, t)| *t == crate::app::git::log::LogTarget::File(i));
                match (hit, app.log_file_tip(i)) {
                    (Some(&(rect, _)), Some(tip)) => (rect, tip),
                    _ => return,
                }
            }
            // 日志列表中延伸到右端仍放不下的提交说明：提示整句。
            Hover::Log(crate::app::git::log::LogTarget::Row(i))
                if app.git.log.clipped == Some(i) =>
            {
                let hit = app
                    .hits
                    .log
                    .iter()
                    .find(|(_, t)| *t == crate::app::git::log::LogTarget::Row(i));
                let commit = app
                    .editor
                    .active()
                    .and_then(|d| d.log.as_deref())
                    .and_then(|v| v.commits.get(i));
                match (hit, commit) {
                    (Some(&(rect, _)), Some(c)) => (rect, c.subject.clone()),
                    _ => return,
                }
            }
            _ => return,
        },
    };
    use crate::app::animation::TipPlace;
    use crate::app::git::log::LogTarget;
    let place = match app.pointer.hover {
        Hover::RulerChange(_) => TipPlace::Beside,
        // 列表的一行：提示从开始悬停时鼠标的位置开始。
        Hover::Log(LogTarget::Row(_) | LogTarget::BranchRow(_) | LogTarget::File(_)) => {
            TipPlace::Pointer
        }
        _ => TipPlace::Below,
    };
    let button = match place {
        TipPlace::Pointer => Rect::new(app.overlays.tooltip.pointer_x, button.y, 0.0, button.h),
        _ => button,
    };
    let tip = (button, label.as_str(), place);
    draw_tip(app, frame, tip, app.tooltip_opacity());
    *app.overlays.tooltip.last.borrow_mut() = Some((button, label, place));
}

/// 画一条提示：对着 `button`，文字 `label`，位置见 [`crate::app::animation::TipPlace`]。整条
/// 按 `opacity` 变淡，用于淡入淡出。
fn draw_tip(
    app: &App,
    frame: &mut Frame<'_>,
    tip: (Rect, &str, crate::app::animation::TipPlace),
    opacity: f32,
) {
    let (button, label, place) = tip;
    frame.with_opacity(opacity, |frame| {
        draw_tip_card(app, frame, button, label, place)
    });
}

fn draw_tip_card(
    app: &App,
    frame: &mut Frame<'_>,
    button: Rect,
    label: &str,
    place: crate::app::animation::TipPlace,
) {
    use crate::app::animation::TipPlace;
    let p = app.palette;
    // 提示分「名称   快捷键」两栏，快捷键以淡色显示在右侧，两栏间距至少 16 逻辑像素。含 `\n` 时
    // 第一段为正文，其余各段为淡色说明（如标签的文件名和所在目录）。
    let (name, key) = match label.split_once(crate::layout::TIP_KEY) {
        Some((name, key)) => (name, Some(key)),
        None => (label, None),
    };
    let size = app.fonts.ui_small;
    let key_w = key.map_or(0.0, |k| 16.0 + frame.measure(FontKind::Ui, size, k));
    // 提示宽度不超过 `TOOLTIP_MAX_WIDTH`，也不超出窗口（左右各留 4 逻辑像素边距）；
    // 文字区再减去左右各 10 逻辑像素的内边距。
    let max_width = TOOLTIP_MAX_WIDTH.min(app.layout.window.w - 8.0);
    let room = max_width - 20.0;
    let lines = tip_lines(name, room, key_w, |piece| {
        frame.measure(FontKind::Ui, size, piece)
    });
    let name_w = lines
        .iter()
        .map(|(line, _)| frame.measure(FontKind::Ui, size, line))
        .fold(0.0, f32::max);
    let width = (name_w + key_w + 20.0).min(max_width);
    // 单行高 26 逻辑像素，每多一行增加 18。
    const LINE: f32 = 18.0;
    let height = 8.0 + LINE * lines.len() as f32;
    let rect = if place == TipPlace::Beside {
        // 概览标尺紧靠窗口右边缘：提示放在标尺左侧、与鼠标同高，不遮挡鼠标所指的标记。
        Rect::new(
            button.x - 8.0 - width,
            button.y + button.h / 2.0 - height / 2.0,
            width,
            height,
        )
    } else {
        let below = button.bottom() + 6.0;
        let y = if below + height > app.layout.window.bottom() {
            button.y - 6.0 - height
        } else {
            below
        };
        // 对着一行时文字从鼠标处开始，对着按钮时居中。
        let x = match place {
            TipPlace::Pointer => button.x - 10.0,
            _ => button.x + (button.w - width) / 2.0,
        };
        let right = (app.layout.window.right() - width - 4.0).max(4.0);
        Rect::new(x.clamp(4.0, right), y, width, height)
    };
    draw_card(frame, p, rect, RADIUS_ROW, p.popup_bg);
    for (i, &(line, body)) in lines.iter().enumerate() {
        let row = Rect::new(rect.x, rect.y + 4.0 + LINE * i as f32, rect.w, LINE);
        let color = if body { p.text } else { p.text_dim };
        ui_text_small(frame, app, rect.x + 10.0, row, line, color, rect);
    }
    if let Some(key) = key {
        let w = frame.measure(FontKind::Ui, size, key);
        let row = Rect::new(rect.x, rect.y + 4.0, rect.w, LINE);
        ui_text_small(
            frame,
            app,
            rect.right() - 10.0 - w,
            row,
            key,
            p.text_dim,
            rect,
        );
    }
}

/// 把提示正文 `name` 按宽度 `room`（逻辑像素）折行，返回各行及其是否属于正文段。
///
/// `name` 以 `\n` 分段，第一段为正文，其余为说明；正文折出的各行都属于正文段。快捷键只占
/// 第一行，因此第一段的宽度再减去 `key_w`。`measure` 返回一段文字的宽度（逻辑像素）。
pub(super) fn tip_lines(
    name: &str,
    room: f32,
    key_w: f32,
    mut measure: impl FnMut(&str) -> f32,
) -> Vec<(&str, bool)> {
    let mut lines = Vec::new();
    for (i, part) in name.split('\n').enumerate() {
        let width = (room - if i == 0 { key_w } else { 0.0 }).max(1.0);
        let wrapped = crate::layout::wrap_text(part, width, &mut measure);
        lines.extend(wrapped.into_iter().map(|line| (line, i == 0)));
    }
    lines
}
