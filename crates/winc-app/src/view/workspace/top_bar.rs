//! 顶栏绘制：标签栏和新建标签按钮，由应用绘制标题栏时右端还有窗口按钮。

use std::path::Path;

use super::*;
use crate::app::TabHit;

/// 绘制顶栏：底色与编辑区相同，与下方的代码区之间不绘制分隔线。
///
/// 标签为胶囊形状，一直排到顶栏右端；右端不放按钮。
pub(in crate::view) fn draw_top_bar(app: &mut App, frame: &mut Frame<'_>) {
    frame.rect(app.layout.top_bar, app.palette.editor_bg);
    draw_tabs(app, frame);
}

/// 绘制顶栏右端的窗口按钮（最小化、最大化或还原、关闭），由应用绘制标题栏时才有。
///
/// 和顶栏上其他按钮一样，悬停时铺浅色；关闭按钮悬停时铺错误色、图标换成白色，提示点击会
/// 关掉窗口。终端面板在右边时按钮落在它的标签栏上。
pub(in crate::view) fn draw_window_controls(app: &App, frame: &mut Frame<'_>) {
    use crate::app::WindowControl;
    if app.layout.controls.w <= 0.0 {
        return;
    }
    let p = app.palette;
    let window = app.layout.window;
    for (button, control) in app.window_control_rects() {
        let hovered = app.hover == Hover::WindowControl(control);
        let close = control == WindowControl::Close;
        let color = if close && hovered {
            frame.rect_clipped(button, p.error, RADIUS_ROW, window);
            p.text_on_accent
        } else {
            draw_button(frame, p, button, window, hovered, false);
            if hovered { p.text } else { p.text_muted }
        };
        let at = center(button);
        match control {
            WindowControl::Minimize => icons::minimize(frame, at, 10.0, color, button),
            WindowControl::Maximize => {
                let restore = app.window_maximized();
                icons::maximize(frame, at, 10.0, restore, color, button);
            }
            WindowControl::Close => icons::close(frame, at, 9.0, color, button),
        }
    }
}

/// 标签的宽度和上面显示的文本：文件名；同名文件的标签在名称后加上级目录名以区分。
///
/// 文本放不下时按标签宽度截短。
struct TabLabel {
    /// 标签的宽度（逻辑像素）。
    width: f32,
    /// 截短后的文件名。
    name: String,
    /// 截短后的上级目录名；仅在存在同名标签、且标签里放得下时有值。
    folder: Option<String>,
    /// 名称是否被截短。
    elided: bool,
}

/// 标签中除文本以外的部分的宽度（逻辑像素）：左内边距 10、文件图标、间隔 7，文本后的间隔 6、
/// 关闭按钮 16、右内边距 6。
const TAB_CHROME: f32 = 10.0 + icons::FILE_WIDTH + 7.0 + 6.0 + 16.0 + 6.0;

/// 上级目录名最多占文本区宽度的比例，文件名优先显示。
const TAB_FOLDER_SHARE: f32 = 0.4;

/// 上级目录名截短后的最小宽度（逻辑像素）；文件名之后剩下的不够该宽度时不显示目录名。
const TAB_FOLDER_MIN: f32 = 24.0;

/// 标签栏两端内容被裁掉时，渐隐区域的宽度（逻辑像素）。
pub(in crate::view) const TAB_FADE: f32 = 24.0;

/// 相邻标签之间竖线的高度（逻辑像素），为标签高度的一半，在标签内垂直居中。
const TAB_DIVIDER: f32 = 14.0;

/// 返回同名文件 `path` 的标签后标注的上级目录，用于与 `others`（其他同名文件）区分。
///
/// 从上一级目录起，不足以区分时逐级向上添加，直到与每个同名文件都不同，如 `a/src` 与
/// `b/src`；无法区分时使用全部上级目录。
pub(crate) fn distinct_folder(path: &Path, others: &[&Path]) -> Option<String> {
    let parents = |p: &Path| -> Vec<String> {
        p.parent()
            .map(|d| d.iter().map(|c| c.to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    };
    let mine = parents(path);
    let theirs: Vec<_> = others.iter().map(|p| parents(p)).collect();
    let suffix = |dirs: &[String], k: usize| dirs[dirs.len().saturating_sub(k)..].join("/");
    let k = (1..=mine.len())
        .find(|&k| theirs.iter().all(|t| suffix(t, k) != suffix(&mine, k)))
        .unwrap_or(mine.len());
    let folder = suffix(&mine, k).trim_start_matches('/').to_owned();
    (!folder.is_empty()).then_some(folder)
}

/// 返回第 `index` 个标签在名称后标注的上级目录，见 [`distinct_folder`]。
///
/// 仅当存在同名的其他标签时返回 `Some`；标签栏与标签列表菜单共用。
pub(crate) fn tab_folder(docs: &[winc_editor::Document], index: usize) -> Option<String> {
    let doc = docs.get(index)?;
    let name = doc.name();
    let others: Vec<_> = docs
        .iter()
        .filter(|d| d.id != doc.id && d.name() == name)
        .filter_map(|d| d.path.as_deref())
        .collect();
    let path = doc.path.as_deref().filter(|_| !others.is_empty())?;
    distinct_folder(path, &others)
}

/// 返回用于选择标签图标的文件名，标签栏与标签列表菜单共用。
///
/// 差异标签取所比较的文件名；其余标签的标题可能带有「· 差异」「· 提交号」等后缀，
/// 按完整标题无法识别类型，因此取「·」之前的部分。
pub(crate) fn tab_icon_name(doc: &winc_editor::Document) -> String {
    doc.diff
        .as_deref()
        .and_then(winc_editor::diff_view::DiffView::file_path)
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            let name = doc.name();
            name.split(" · ").next().unwrap_or(&name).to_owned()
        })
}

/// 计算每个标签的宽度和显示文本，顺序与文档列表一致。
///
/// 宽度贴合文本，介于 [`TAB_WIDTH_MIN`] 和 [`TAB_WIDTH_MAX`] 之间，取整到整像素，文本更长时
/// 截短；用鼠标关闭标签后，移到关掉的位置上的标签暂用锁定的宽度（见 `TabBar::width_lock`）。
fn tab_labels(app: &App, frame: &mut Frame<'_>) -> Vec<TabLabel> {
    let docs = &app.editor.documents;
    // 只有同名的标签需要标注上级目录：先数出各名字的标签数，名字唯一的标签不再与其余全部标签
    // 逐一比较（每帧都要计算，标签多时是标签数的平方）。
    let names: Vec<_> = docs.iter().map(|d| d.name()).collect();
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for name in &names {
        *counts.entry(name.as_ref()).or_default() += 1;
    }

    docs.iter()
        .enumerate()
        .map(|(index, _)| {
            let name: &str = &names[index];
            let folder = (counts[name] > 1)
                .then(|| tab_folder(docs, index))
                .flatten();

            let mut text = frame.measure(FontKind::Ui, app.fonts.ui, name);
            if let Some(folder) = &folder {
                text += 6.0 + frame.measure(FontKind::Ui, app.fonts.ui_small, folder);
            }
            let width = match app.tabs.width_lock {
                Some((locked, width)) if locked == index => width,
                _ => (TAB_CHROME + text)
                    .ceil()
                    .clamp(TAB_WIDTH_MIN, TAB_WIDTH_MAX),
            };
            let room = width - TAB_CHROME;

            let Some(folder) = folder else {
                let shown = ellipsize(frame, name, app.fonts.ui, room).into_owned();
                return TabLabel {
                    width,
                    elided: shown != name,
                    name: shown,
                    folder: None,
                };
            };

            // 目录名最多占文本区的 `TAB_FOLDER_SHARE`，剩下的给文件名；文件名截短后若有余量，
            // 目录名用满余量。
            let folder_w = frame
                .measure(FontKind::Ui, app.fonts.ui_small, &folder)
                .min(room * TAB_FOLDER_SHARE);
            let shown = ellipsize(frame, name, app.fonts.ui, room - 6.0 - folder_w).into_owned();
            let left = room - 6.0 - frame.measure(FontKind::Ui, app.fonts.ui, &shown);
            // 放得下完整目录名时直接显示（`src`、`api` 等短名比 `TAB_FOLDER_MIN` 还窄）；
            // 须截短时至少保留 `TAB_FOLDER_MIN`。
            let full = frame.measure(FontKind::Ui, app.fonts.ui_small, &folder);
            let folder = (left >= full.min(TAB_FOLDER_MIN))
                .then(|| ellipsize(frame, &folder, app.fonts.ui_small, left).into_owned());
            TabLabel {
                width,
                elided: shown != name,
                name: shown,
                folder,
            }
        })
        .collect()
}

/// 标签放不下时，新建按钮和最右端「⌄」之间的间距（逻辑像素）。
const TAB_LIST_GAP: f32 = 2.0;

/// 绘制标签栏：当前标签是实心胶囊，相邻两个未选中标签之间绘制一条竖线；有未保存修改的标签
/// 显示圆点，悬停时变成关闭按钮。
///
/// 放不下时可以横向滚动，被裁掉的一端渐隐，新建按钮和列出全部标签的「⌄」固定在右端。
/// 同时更新 `app.tabs` 中的几何信息和各标签的命中区域。
fn draw_tabs(app: &mut App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let area = app.layout.tabs;
    let top = area.y + (area.h - TAB_HEIGHT) / 2.0;
    let button_y = area.y + (area.h - BUTTON) / 2.0;

    app.note_opened_tabs();
    let mut labels = tab_labels(app, frame);
    // 刚打开的标签按长出来的程度算宽度：右边的标签、新建按钮跟着逐步让开
    // （`App::note_opened_tabs`）。
    let grow: Vec<f32> = app
        .editor
        .documents
        .iter()
        .map(|d| app.tab_grow(d.id))
        .collect();
    for (label, &g) in labels.iter_mut().zip(&grow) {
        if g < 1.0 {
            label.width *= g;
        }
    }
    // 第 `i` 个标签的左边缘相对第一个标签的偏移；最后一项是全部标签排开的总宽度。
    let offsets: Vec<f32> = std::iter::once(0.0)
        .chain(labels.iter().scan(0.0, |x, l| {
            *x += l.width + TAB_GAP;
            Some(*x)
        }))
        .collect();
    let content = offsets[labels.len()];
    let overflow = content + BUTTON > area.w;
    // 放不下时最右端显示「⌄」，点击后列出全部标签，使滚出可见范围的标签也能直接切换；
    // 新建按钮位于其左侧。
    let list = overflow.then(|| Rect::new(area.right() - BUTTON, button_y, BUTTON, BUTTON));
    let view = match list {
        Some(list) => Rect::new(
            area.x,
            area.y,
            (list.x - TAB_LIST_GAP - BUTTON - 6.0 - area.x).max(0.0),
            area.h,
        ),
        None => area,
    };
    // 新建按钮跟着最后一个标签走：关掉一个标签、右边的还在滑回来时，它和最后一个标签一起滑，
    // 不先跳到终点压在还没滑到的标签上；关掉的是最后一个时，跟着它收窄的右边往回走。
    let trailing = {
        let last = app
            .editor
            .documents
            .last()
            .map_or(0.0, |d| app.tab_slide(d.id));
        let ghosts = app.tabs.closing.iter().map(|g| {
            let shrink = g.width * (1.0 - g.clock.progress());
            g.left + shrink + TAB_GAP - content
        });
        ghosts.fold(last, f32::max).max(0.0)
    };
    let new_tab = if overflow {
        Rect::new(view.right() + 6.0, button_y, BUTTON, BUTTON)
    } else {
        Rect::new(area.x + content + trailing, button_y, BUTTON, BUTTON)
    };

    let tabs = &mut app.tabs;
    tabs.view = view;
    tabs.new_tab = new_tab;
    tabs.list = list;
    tabs.content = content;

    // 当前标签改变后，将该标签滚动到可见范围内，并在两侧留出渐隐区域的宽度。
    if let Some(active) = app.editor.active {
        let id = app.editor.documents[active].id;

        if tabs.revealed != Some(id) {
            let start = offsets[active];
            let end = start + labels[active].width;
            let margin = if overflow { TAB_FADE } else { 0.0 };

            if start - margin < tabs.scroll {
                tabs.scroll = start - margin;
            } else if end + margin > tabs.scroll + view.w {
                tabs.scroll = end + margin - view.w;
            }

            tabs.revealed = Some(id);
        }
    }

    tabs.scroll = tabs.scroll.clamp(0.0, tabs.max_scroll());
    tabs.widths.clear();
    tabs.widths.extend(labels.iter().map(|l| l.width));
    // 绘制时滚动位置取整到物理像素：触控板滚动产生小数位移，文字按不同的亚像素位置光栅化时
    // 会逐帧变化，表现为闪烁和模糊。
    let scroll = (tabs.scroll * frame.scale()).round() / frame.scale();
    let dragged = app.dragged_tab();
    // 每帧重新生成命中区域，并复用上一帧的 `hits` 内存。
    let mut hits = std::mem::take(&mut app.tabs.hits);
    hits.clear();
    let mut x = view.x - scroll;
    let mut lifted = None;
    // 当前、悬停和拖动中的标签有自己的底色，两侧不绘制竖线；拖动中的标签离开了原位，原位两侧的
    // 竖线也不绘制。
    let hovered = match app.hover {
        Hover::Tab(index, _) => Some(index),
        _ => None,
    };
    let plain = |index: usize| {
        Some(index) != app.editor.active
            && Some(index) != hovered
            && dragged.is_none_or(|(id, _)| id != app.editor.documents[index].id)
    };

    // 刚关闭的标签先绘制：内容在原宽度内淡出，宽度在淡出结束后才收起，收起途中不显示内容。
    for ghost in &app.tabs.closing {
        let opacity = ghost.fade.fade_out();
        if opacity <= 0.0 {
            continue;
        }
        let tab = Rect::new(view.x - scroll + ghost.left, top, ghost.width, TAB_HEIGHT);
        let clip = tab.intersect(view);
        frame.with_opacity(opacity, |frame| {
            if ghost.active {
                let full = Rect::new(tab.x, tab.y, ghost.width, tab.h);
                draw_tab_fill(frame, p, full, true, false, clip);
            }
            let icon = (tab.x + 10.0, icon_center_y(app, tab, app.fonts.ui));
            let text_x = if ghost.log {
                let c = (icon.0 + icons::FILE_WIDTH / 2.0, icon.1);
                icons::git(frame, c, 1.0, p.accent, clip);
                icon.0 + icons::FILE_WIDTH + 7.0
            } else {
                draw_file_icon(frame, p, icon, &ghost.icon, clip) + 7.0
            };
            let color = if ghost.active { p.text } else { p.text_muted };
            ui_text(frame, app, text_x, tab, &ghost.name, color, clip);
        });
    }

    for (index, (doc, label)) in app.editor.documents.iter().zip(&labels).enumerate() {
        let slot = x;
        x += label.width + TAB_GAP;

        // 被拖动的标签跟随鼠标，最后绘制，压在其他标签之上；其他标签刚让位时仍在滑动动画中。
        let left = match dragged {
            Some((id, left)) if id == doc.id => {
                lifted = Some((index, view.x - scroll + left));
                continue;
            }
            _ => slot + app.tab_slide(doc.id),
        };
        let tab = Rect::new(left, top, label.width, TAB_HEIGHT);
        if tab.right() < view.x || tab.x > view.right() {
            continue;
        }

        // 竖线跟随右侧标签绘制在其左边的间距正中，让位动画中随标签一起滑动。
        if index > 0 && plain(index) && plain(index - 1) {
            let line = Rect::new(
                left - TAB_GAP / 2.0 - 0.5,
                top + (TAB_HEIGHT - TAB_DIVIDER) / 2.0,
                1.0,
                TAB_DIVIDER,
            );
            frame.rect_clipped(line, p.tab_divider, 0.0, view);
        }

        // 展开途中按最终宽度排列图标、名字和关闭按钮，裁剪到当前宽度；内容在宽度展开大部分后才
        // 淡入（`reveal`），不显示被裁去大半的名字。
        let g = grow[index];
        if g < 1.0 {
            let clip = Rect::new(tab.x, view.y, tab.w, view.h).intersect(view);
            let full = Rect::new(tab.x, tab.y, tab.w / g.max(0.01), tab.h);
            frame.with_opacity(reveal(g), |frame| {
                let mut hit = draw_tab(app, frame, index, full, label, clip);
                hit.tab = tab;
                hits.push(hit);
            });
            continue;
        }
        hits.push(draw_tab(app, frame, index, tab, label, view));
    }

    if let Some((index, left)) = lifted {
        let tab = Rect::new(left, top, labels[index].width, TAB_HEIGHT);
        hits.push(draw_tab(app, frame, index, tab, &labels[index], view));
    }

    let fade_h = Rect::new(view.x, top, TAB_FADE, TAB_HEIGHT);

    // 渐隐的强度随离两端的距离在 `TAB_FADE` 内线性变化，滚到两端附近时不突然出现或消失。
    let left = (scroll / TAB_FADE).min(1.0);
    if left > 0.0 {
        frame.with_opacity(left, |frame| fade(frame, fade_h, p.editor_bg, true));
    }

    let rest = ((app.tabs.max_scroll() - scroll) / TAB_FADE).min(1.0);
    if rest > 0.0 {
        let right = Rect::new(view.right() - TAB_FADE, top, TAB_FADE, TAB_HEIGHT);
        frame.with_opacity(rest, |frame| fade(frame, right, p.editor_bg, false));
    }

    draw_button(frame, p, new_tab, area, app.hover == Hover::NewTab, false);
    icons::plus(frame, center(new_tab), 9.0, p.text_muted, area);
    if let Some(list) = list {
        // 菜单打开时按钮显示为按下状态。
        let open = app.menu_button == Some(MenuButton::TabList);
        let hovered = app.hover == Hover::TabList;
        draw_button(frame, p, list, area, hovered, open);
        let color = if hovered || open {
            p.text
        } else {
            p.text_muted
        };
        icons::chevron(
            frame,
            center(list),
            8.0,
            icons::Direction::Down,
            color,
            area,
        );
    }
    app.tabs.hits = hits;
    draw_preview_button(app, frame);
    draw_browser_button(app, frame);
}

/// 绘制标签栏右端的「在浏览器中打开」按钮（`Layout::browser`）：出现、消失时随按钮宽度淡入
/// 淡出并缩放，同预览切换。
fn draw_browser_button(app: &App, frame: &mut Frame<'_>) {
    let button = app.layout.browser;
    if button.w <= 0.0 {
        return;
    }
    let p = app.palette;
    let present = app.browser_page().is_some();
    let share = app.module_fx.browser.share(present);
    let opacity = app.module_fx.browser.opacity(present);
    let hovered = present && app.hover == Hover::Browser;
    // 底色与图标一同淡入淡出，宽度变化途中不显示随按钮移动的底色。
    frame.with_opacity(opacity, |frame| {
        draw_button(frame, p, button, app.layout.top_bar, hovered, false);
        let color = if hovered { p.text } else { p.text_muted };
        let clip = button.intersect(app.layout.top_bar);
        icons::globe(frame, center(button), 0.6 + 0.4 * share, color, clip);
    });
}

/// 绘制标签栏右端的「源代码 / 分栏 / 预览」三段切换，当前模式的按钮显示按下底色和正文色图标。
///
/// 在可预览与不可预览的文件之间切换时（`App::track_modules`），按钮按比例收窄或展开
/// （`Layout::preview`），图标随之淡入淡出并缩放，只绘制在按钮内；收窄途中按下状态沿用
/// 最后一帧的模式，且不响应点击。
fn draw_preview_button(app: &mut App, frame: &mut Frame<'_>) {
    use winc_editor::PreviewMode::*;
    let group = app.layout.preview;
    let present = app.editor.active().is_some_and(|d| d.can_preview());
    if present {
        app.module_fx.preview_mode = app.editor.active().map(|d| d.preview_mode());
    }
    let Some(current) = app.module_fx.preview_mode.filter(|_| group.w > 0.0) else {
        return;
    };
    let p = app.palette;
    let share = app.module_fx.preview.share(present);
    let opacity = app.module_fx.preview.opacity(present);
    let segments = crate::layout::preview_segments(group);
    let hover = app.hover;
    // 按下底色与图标一同淡入淡出，宽度变化途中不显示随按钮移动的底色。
    frame.with_opacity(opacity, |frame| {
        for (mode, button) in [Source, Split, Preview].into_iter().zip(segments) {
            let hovered = present && hover == Hover::Preview(mode);
            let on = current == mode;
            draw_button(frame, p, button, app.layout.top_bar, hovered, on);
            let color = if on || hovered { p.text } else { p.text_muted };
            let k = 0.6 + 0.4 * share;
            let clip = button.intersect(app.layout.top_bar);
            match mode {
                Source => icons::code_scaled(frame, center(button), k, color, clip),
                Split => icons::split_scaled(frame, center(button), k, color, clip),
                Preview => icons::eye(frame, center(button), k, color, clip),
            }
        }
    });
}

/// 绘制第 `index` 个标签（占据 `tab` 矩形，裁剪到 `view`），返回其命中区域。
fn draw_tab(
    app: &App,
    frame: &mut Frame<'_>,
    index: usize,
    tab: Rect,
    label: &TabLabel,
    view: Rect,
) -> TabHit {
    let p = app.palette;
    let doc = &app.editor.documents[index];
    let active = app.editor.active == Some(index);
    let hovered = matches!(app.hover, Hover::Tab(i, _) if i == index);
    // 多选里不是当前的标签：底色是当前标签的一半深，文字用当前标签的颜色。
    let marked = !active && app.tabs_multi_selected() && app.tab_selection().contains(&doc.id);

    // 当前的是一块带细边的底（颜色由主题定：带一点强调色，其余是凸起的一块灰），悬停只铺浅色。
    if marked {
        frame.rect_clipped(tab, p.tab_fill.alpha(p.tab_fill.a * 0.5), RADIUS_TAB, view);
    } else {
        draw_tab_fill(frame, p, tab, active, hovered, view);
    }

    // 图标按 `tab_icon_name` 取；日志标签绘制 Git 的图标。
    let icon_name = tab_icon_name(doc);
    let icon = (tab.x + 10.0, icon_center_y(app, tab, app.fonts.ui));
    let cx = if doc.log.is_some() {
        let c = (icon.0 + icons::FILE_WIDTH / 2.0, icon.1);
        // 实色的强调色，与文件标签的彩色图标同样醒目。
        icons::git(frame, c, 1.0, p.accent, view);
        icon.0 + icons::FILE_WIDTH + 7.0
    } else {
        draw_file_icon(frame, p, icon, &icon_name, view) + 7.0
    };
    let color = if active || marked {
        p.text
    } else {
        p.text_muted
    };
    // 预览标签（见 `App::preview_index`）的名称倾斜，与普通标签区分。
    let cx = if app.preview_index() == Some(index) {
        ui_text_italic(frame, app, cx, tab, &label.name, color, view)
    } else {
        ui_text(frame, app, cx, tab, &label.name, color, view)
    };

    if let Some(folder) = &label.folder {
        ui_text_small(frame, app, cx + 6.0, tab, folder, p.text_dim, view);
    }

    let close = tab_close(tab);
    if doc.is_dirty() && !hovered {
        let dot = Rect::new(close.x + 5.0, close.y + 5.0, 6.0, 6.0);
        frame.rect_clipped(dot, p.modified_dot, 3.0, view);
    } else if hovered || active {
        draw_tab_close(frame, p, close, app.hover == Hover::Tab(index, true), view);
    }

    TabHit {
        index,
        tab,
        close,
        elided: label.elided,
    }
}

/// 标签的底（编辑区和终端面板的标签共用），都不带边：当前的是一块实底，颜色由主题定，带一点
/// 强调色；悬停只铺浅色。
///
/// 选中与悬停的区分依靠实底与浅色，而非两种相近的灰色。
pub(in crate::view) fn draw_tab_fill(
    frame: &mut Frame<'_>,
    p: &Palette,
    tab: Rect,
    active: bool,
    hovered: bool,
    clip: Rect,
) {
    if active {
        frame.rect_clipped(tab, p.tab_fill, RADIUS_TAB, clip);
    } else if hovered {
        frame.rect_clipped(tab, p.hover, RADIUS_TAB, clip);
    }
}

/// 标签的关闭按钮：固定在标签右端，所有标签的关闭按钮位置一致。
pub(in crate::view) fn tab_close(tab: Rect) -> Rect {
    let y = tab.y + (tab.h - 16.0) / 2.0;
    Rect::new(tab.right() - 6.0 - 16.0, y, 16.0, 16.0)
}

/// 绘制关闭按钮；`on` 为真（鼠标在按钮上）时垫一个圆底。
///
/// 按钮的圆形直径为 16 逻辑像素，比其他按钮小，与标签的胶囊形状协调。
pub(in crate::view) fn draw_tab_close(
    frame: &mut Frame<'_>,
    p: &Palette,
    close: Rect,
    on: bool,
    clip: Rect,
) {
    if on {
        frame.rect_clipped(close, p.hover, close.h / 2.0, clip);
    }
    let color = if on { p.text_muted } else { p.text_dim };
    icons::close(frame, center(close), 6.5, color, clip);
}
