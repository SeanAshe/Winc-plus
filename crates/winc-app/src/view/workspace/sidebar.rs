//! 侧栏绘制：第二行的项目和分支、文件树工具栏和「⋯」菜单、文件树、新建条目的输入行。
//!
//! 纵向和横向滚动、固定在顶部的目录，以及放不下的名称在编辑区上方的延续绘制都在这里处理；
//! 每一行的矩形和点击命中由 `app::tree` 在绘制时记录。

use std::borrow::Cow;
use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};

use winc_editor::git::Change;
use winc_editor::{Entry, EntryKind};
use winc_i18n::t;

use super::*;
use crate::app::animation::stage;
use crate::app::{DropZone, Focus, LabelCause, LabelView, NameInput, TreeMotion};
use crate::icons::Direction;
use crate::layout::{
    SidebarView, TREE_NAME_MIN, TreeTool, folding_tools, hscrollbar_thumb, tree_row_slot,
    tree_tools,
};
use crate::theme::SCROLLBAR_HIT;

/// 绘制侧栏：
///
/// - 顶栏（窗口按钮和侧栏开关右边）：视图切换图标（文件、搜索、提交），见
///   [`panel::draw_view_tabs`]；
/// - 第二行：左边是项目和分支的上下文栏（[`draw_context_bar`]，三个视图都一样），
///   右端是当前视图的工具按钮；
/// - 下面是当前视图：文件树（底部是可以展开的大纲，见 [`outline::draw_outline`]）、全局搜索
///   面板或提交视图。
pub(in crate::view) fn draw_sidebar(app: &mut App, frame: &mut Frame<'_>) {
    panel::draw_view_tabs(app, frame);

    if app.sidebar.view == SidebarView::Search {
        let right = app.panel_parts().tools[0].0.x - 4.0;
        let bar = ContextBar::new(app, frame);
        draw_context_bar(app, frame, &bar, right);
        if let Some(caret) = panel::draw_panel(app, frame) {
            app.ime.cursor = Some(caret);
        }

        return;
    }

    if app.sidebar.view == SidebarView::Commit {
        draw_commit_view(app, frame);
        return;
    }

    draw_project_header(app, frame);
    draw_tree(app, frame);
    outline::draw_outline(app, frame);
}

/// 绘制提交视图：工作区改动，或从日志打开的差异的文件列表（见 `App::with_log_list`）。
///
/// 两者切换时列表直接替换，只有标题行右端过渡（[`App::log_tree_share`]）：前一半时间里离开
/// 一方的按钮（或提交号）淡出，后一半时间里目标一方的淡入，两者位置重叠，不同时显示；左侧
/// 项目栏的右端随之伸缩。
fn draw_commit_view(app: &mut App, frame: &mut Frame<'_>) {
    let share = app.log_tree_share();
    let version = (2.0 * share - 1.0).clamp(0.0, 1.0);
    let tools = (1.0 - 2.0 * share).clamp(0.0, 1.0);
    // 项目栏的右端：提交视图在工具按钮（或位置按钮）之前，文件列表在提交号之前；上下文栏
    // 自己会在右边留出 4 的间距。
    let commit_right = app.with_log_tree_shown(false, |app| {
        let tools = app.commit_parts().tools[0].0.x - 4.0;
        git::commit::location_span(app, frame).map_or(tools, |(left, _)| left)
    });
    let right = match log_version_left(app, frame) {
        Some(left) => commit_right + (left - 4.0 - commit_right) * share,
        None => commit_right,
    };
    let bar = ContextBar::new(app, frame);
    draw_context_bar(app, frame, &bar, right);

    if app.log_diff_tree().is_some() {
        if tools > 0.0 {
            app.with_log_tree_shown(false, |app| {
                git::commit::draw_commit_tools(app, frame, tools);
            });
        }
        app.with_log_list(|app| git::commit::draw_commit_browse(app, frame));
        frame.with_opacity(version, |frame| draw_log_diff_version(app, frame, version));
    } else {
        if version > 0.0 {
            app.with_log_tree_shown(true, |app| {
                frame.with_opacity(version, |frame| draw_log_diff_version(app, frame, version));
            });
        }
        if let Some(caret) = git::commit::draw_commit(app, frame, tools) {
            app.ime.cursor = Some(caret);
        }
    }
}

/// 返回文件列表标题行中版本（提交号）的左边；没有文件列表时返回 `None`。与
/// [`draw_log_diff_version`] 使用同一套排法。
fn log_version_left(app: &App, frame: &mut Frame<'_>) -> Option<f32> {
    let version = app.log_tree_version()?;
    let header = app.layout.sidebar_header;
    let right = app.log_diff_layout_button().x - 4.0;
    let version = truncate_end(
        frame,
        version,
        app.fonts.ui_small,
        (header.w / 2.0).max(0.0),
    );
    Some(right - frame.measure(FontKind::Ui, app.fonts.ui_small, &version))
}

/// 在侧栏标题行右端、提交视图工具按钮的位置绘制切换树形与列表的按钮和文件列表的版本
/// （提交号），返回版本的左边。
///
/// 按钮显示切换后的显示方式。`reveal` 是文件列表的露出比例：切换过程中版本从右侧滑入。
fn draw_log_diff_version(app: &mut App, frame: &mut Frame<'_>, reveal: f32) -> f32 {
    let Some(version) = app.log_tree_version().map(str::to_owned) else {
        return app.layout.sidebar_header.right();
    };
    let p = app.palette;
    let header = app.layout.sidebar_header;
    let button = app.log_diff_layout_button();
    let hovered = app.hover == Hover::LogDiffLayout;
    draw_button(frame, p, button, header, hovered, false);
    let color = if hovered { p.text } else { p.text_muted };
    if app.log_list().flat() {
        icons::tree_view(frame, center(button), color, header);
    } else {
        icons::list_view(frame, center(button), color, header);
    }
    let right = button.x - 4.0;
    let room = (header.w / 2.0).max(0.0);
    let version = truncate_end(frame, &version, app.fonts.ui_small, room);
    let width = frame.measure(FontKind::Ui, app.fonts.ui_small, &version);
    let left = right - width;
    let x = left + (1.0 - reveal) * 8.0;
    ui_text_small(frame, app, x, header, &version, p.text_dim, header);
    left
}

/// 绘制文件视图的第二行：左边是上下文栏（项目、当前分支），右端是文件树工具栏。
fn draw_project_header(app: &mut App, frame: &mut Frame<'_>) {
    let header = app.layout.sidebar_header;
    let bar = ContextBar::new(app, frame);

    // 侧栏变窄时先收工具栏、后截名字：为完整的项目名留出宽度（分支不占宽度：放不下时不再绘制，
    // 状态栏里始终显示），工具栏放不下时从右向左把按钮收进「⋯」菜单；只剩「⋯」时才截短名字，
    // 项目名截到最小宽度（`TREE_NAME_MIN`）为止，它是切换目录的入口，必须始终可点击。
    (app.hits.tree_tools, app.hits.tree_more) = if bar.opened {
        let only_more = header.right() - 8.0 - BUTTON_SMALL - 4.0;
        let name_right = (bar.left + bar.project_full() + 8.0)
            .min(only_more)
            .max(bar.left + bar.name_offset() + TREE_NAME_MIN);
        tree_tools(header, name_right)
    } else {
        (Vec::new(), None)
    };
    app.step_tool_fold();
    let right = app
        .hits
        .tree_tools
        .first()
        .map(|(button, _)| button.x)
        .or(app.hits.tree_more.map(|more| more.x))
        .map_or(header.right() - 8.0, |x| x - 4.0);

    draw_context_bar(app, frame, &bar, right);
    draw_tree_tools(app, frame);
}

/// 文件树中目录有 Git 改动时，右端圆点的直径（逻辑像素）。
const GIT_DOT: f32 = 6.0;
/// 文件树中放不下的名称在侧栏边缘淡出的宽度（逻辑像素），
/// 与结果行上按钮左侧的渐隐同宽。
pub(in crate::view) const NAME_FADE: f32 = 20.0;
/// 项目按钮里徽章占的宽度（逻辑像素）：左边 6、徽章 20、右边 8。
pub(in crate::view) const BADGE_ROOM: f32 = 34.0;
/// 徽章中心相对项目按钮左边的距离（逻辑像素）。
pub(in crate::view) const BADGE_CENTER: f32 = 16.0;

/// 侧栏第二行左边的上下文栏：现在在哪个项目。
///
/// 项目按钮三个视图（文件、搜索、提交）都一样，切换视图时不动：首字母徽章、项目名和下拉箭头，
/// 点了弹项目菜单（最近的目录、打开目录、关闭目录）。
pub(in crate::view) struct ContextBar {
    /// 项目名；没有打开目录时是「未打开目录」。
    project: String,
    /// 打开了目录（可以绘制徽章，名称用正文色）。
    opened: bool,
    /// 项目按钮的左边。
    left: f32,
    /// 项目名完整显示时的文字宽度。
    project_w: f32,
}

impl ContextBar {
    /// 读出当前的项目名，并量出完整显示时的文字宽度。
    pub(in crate::view) fn new(app: &App, frame: &mut Frame<'_>) -> Self {
        let opened = app.editor.workspace.is_some();
        let project = app
            .project_name()
            .unwrap_or_else(|| t!("workspace-sidebar-no-folder").to_owned());
        let project_w = frame.measure(FontKind::Ui, app.fonts.ui, &project);
        ContextBar {
            project,
            opened,
            left: app.layout.sidebar_header.x + 8.0,
            project_w,
        }
    }

    /// 项目名文字相对项目按钮左边的偏移：打开了目录时让出徽章（徽章一直在，名字放不下时
    /// 截名字，不省徽章），否则留 6。
    fn name_offset(&self) -> f32 {
        if self.opened { BADGE_ROOM } else { 6.0 }
    }

    /// 项目按钮的左边。
    pub(in crate::view) fn left(&self) -> f32 {
        self.left
    }

    /// 项目按钮完整显示时的宽度。
    pub(in crate::view) fn project_full(&self) -> f32 {
        self.name_offset() + self.project_w + 18.0
    }
}

/// 在第二行左边绘制上下文栏（[`ContextBar`]），右边不超过 `right`；记下项目按钮的位置
/// （`project_hit`）。
pub(super) fn draw_context_bar(app: &mut App, frame: &mut Frame<'_>, bar: &ContextBar, right: f32) {
    let p = app.palette;
    let header = app.layout.sidebar_header;
    let clip = header.intersect(app.layout.sidebar);
    let room = (right - 4.0 - bar.left).max(0.0);
    // 留 0.5 的余量：刚好放得下时，浮点运算的误差不应该让名字被截短。
    let fits = bar.project_full() <= room + 0.5;
    let project_w = if fits { bar.project_full() } else { room };
    let name_x = bar.left + bar.name_offset();

    // 项目：徽章一直在；名称太长时从末尾截短，其后是下拉箭头。
    let badge_rect = Rect::new(
        bar.left + 6.0,
        header.y + (header.h - 20.0) / 2.0,
        20.0,
        20.0,
    );
    let name_max = if fits {
        f32::INFINITY
    } else {
        (bar.left + project_w - 18.0 - name_x).max(0.0)
    };
    let name = truncate_end(frame, &bar.project, app.fonts.ui, name_max);
    let name_end = name_x + frame.measure(FontKind::Ui, app.fonts.ui, &name);
    let hit = Rect::new(
        bar.left,
        header.y + (header.h - BUTTON_SMALL) / 2.0,
        name_end + 18.0 - bar.left,
        BUTTON_SMALL,
    );
    let open = app.menu_button == Some(MenuButton::Project);
    draw_button(frame, p, hit, clip, app.hover == Hover::ProjectName, open);
    if bar.opened {
        draw_badge(app, frame, badge_rect, &bar.project, clip);
    }
    let color = if bar.opened { p.text } else { p.text_muted };
    let x = ui_text(frame, app, name_x, header, &name, color, clip);
    let arrow = (x + 9.0, icon_center_y(app, header, app.fonts.ui));
    icons::chevron(frame, arrow, 6.5, Direction::Down, p.text_muted, clip);
    app.hits.project = hit.intersect(clip);
}

/// 绘制项目名前面的徽章：圆角方块内是项目名的前两个字母或数字（大写），没有时用「W」。
///
/// 打开的是附属工作树时，徽章用主工作树的颜色（同一个仓库的工作树颜色相同），右下角加一个工作树
/// 记号。
fn draw_badge(app: &App, frame: &mut Frame<'_>, rect: Rect, project: &str, header: Rect) {
    let main = app.git.main_worktree();
    let color = match main {
        Some(main) => app.state.badge_color(&main.path),
        None => app
            .editor
            .root()
            .map_or(0, |root| app.state.badge_color(root)),
    };
    let badge = app.palette.project_badge(project, color);
    frame.rect_clipped(rect, badge.background, 6.0, header);

    let w = frame.measure(FontKind::Ui, app.fonts.initials, &badge.initials);
    caps_label(
        frame,
        app,
        app.fonts.initials,
        (rect.x + (rect.w - w) / 2.0, rect),
        &badge.initials,
        badge.text,
        header,
    );
    if main.is_some() {
        let colors = (app.palette.sidebar_bg, badge.background, badge.text);
        icons::worktree_mark(
            frame,
            (rect.right() - 1.0, rect.bottom() - 1.0),
            colors,
            header,
        );
    }
}

/// 绘制项目名右侧的文件树工具栏：新建文件、新建目录、刷新、全部折叠、定位当前文件；
/// 放不下的按钮收进「⋯」按钮。
///
/// 按收起动画的进度排布（`folding_tools`）：收起的按钮滑进「⋯」并淡出，展开时反过来；
/// 动画途中压到项目名上的部分不绘制。
fn draw_tree_tools(app: &App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let header = app.layout.sidebar_header;
    let left = app.hits.project.right() + BUTTON_GAP;
    let clip = Rect::new(left, header.y, (header.right() - left).max(0.0), header.h);
    let (shown, more_shown) = app.tool_fold.eased();
    let (tools, more) = folding_tools(header, &shown, more_shown);

    for (button, tool, shown) in tools {
        let kept = app.hits.tree_tools.iter().any(|&(_, t)| t == tool);
        let hovered = kept && app.hover == Hover::TreeTool(tool);
        let color = if hovered { p.text } else { p.text_muted };
        // 透明度比位置变化得快：滑到「⋯」上时已经接近透明，几个图标叠在一起也不会混成一片。
        frame.with_opacity(shown * shown, |frame| {
            draw_button(frame, p, button, clip, hovered, false);
            draw_tree_tool_icon(frame, tool, center(button), color, button.intersect(clip));
        });
    }

    if let Some(more) = more {
        // 菜单打开时按钮显示为按下状态。
        let open = app.menu_button == Some(MenuButton::TreeMore);
        let hovered = app.hits.tree_more.is_some() && app.hover == Hover::TreeMore;
        let color = if hovered || open {
            p.text
        } else {
            p.text_muted
        };
        frame.with_opacity(more_shown * more_shown, |frame| {
            draw_button(frame, p, more, clip, hovered, open);
            icons::more(frame, center(more), color, more.intersect(clip));
        });
    }
}

/// 绘制工具 `tool` 的图标，中心在 `center`，裁剪到 `clip`。
fn draw_tree_tool_icon(
    frame: &mut Frame<'_>,
    tool: TreeTool,
    center: (f32, f32),
    color: Color,
    clip: Rect,
) {
    match tool {
        TreeTool::NewFile => icons::add_file(frame, center, color, clip),
        TreeTool::NewFolder => icons::add_folder(frame, center, color, clip),
        TreeTool::Refresh => icons::refresh(frame, center, color, clip),
        TreeTool::CollapseAll => icons::collapse_all(frame, center, color, clip),
        TreeTool::Locate => icons::locate(frame, center, color, clip),
    }
}

/// 绘制文件树：选中行和悬停行是内缩的圆角块。
fn draw_tree(app: &mut App, frame: &mut Frame<'_>) {
    let animate = app.animate_tree_labels();
    app.tree.labels.begin_frame(animate);
    let p = app.palette;
    let tree = app.layout.tree;

    let Some(ws) = app.editor.workspace.as_ref() else {
        // 顶部已经显示「未打开目录」，这里只提示打开方式，并与该文本左对齐。
        let row = Rect::new(tree.x, tree.y + 4.0, tree.w, TREE_ROW);
        let x = app.layout.sidebar_header.x + 14.0;
        // 只有 macOS 的选择器能同时选文件和目录，其他平台上提示打开目录的快捷键。
        let hint = if cfg!(target_os = "macos") {
            match keymap::shortcut(Action::OpenFile) {
                Some(key) => t!("workspace-tree-open-hint-key", key = key),
                None => t!("workspace-tree-open-hint").to_owned(),
            }
        } else {
            match keymap::shortcut(Action::OpenFolder) {
                Some(key) => t!("workspace-tree-open-folder-hint-key", key = key),
                None => t!("workspace-tree-open-folder-hint").to_owned(),
            }
        };
        ui_text_small(frame, app, x, row, &hint, p.text_dim, tree);
        return;
    };

    let rows = ws.rows();

    // 目录为空时居中显示一句提示，说明新建文件的方式，与搜索、提交视图的空列表相同；
    // 双击空白处即在根目录新建文件（见 `click_tree`）。正在输入新建的名称时不显示提示。
    let creating = app.name_input.as_ref().is_some_and(|e| e.rename.is_none());
    if rows.is_empty() && !creating {
        let span = (tree, sidebar_span(app));
        let (title, hint) = (t!("workspace-tree-empty"), t!("workspace-tree-empty-hint"));
        draw_empty_state(app, frame, span, title, Some(hint));
    }

    // 新建文件、目录时，在目标目录子树的第一行插入一行输入名称的输入行；重命名时输入框
    // 绘制在原条目所在的行上，不另插一行。输入框先从 `app` 中取出，绘制后放回，
    // 这样循环中读取 `app` 时不需要复制整棵文件树。
    let mut name_input = app.name_input.take();
    let editing = name_input.as_ref().map(|e| e.index);
    let inserted = name_input
        .as_ref()
        .filter(|e| e.rename.is_none())
        .map(|e| e.index);
    let mut ime = None;

    // 目录展开或收起的动画中，按展开后的行序排布：收起时把正在收起的子行插回原位。子行区域
    // 停在原位，只露出 `reveal` 所示比例的部分；其下的行向上移动 `hidden`（逻辑像素）的高度。
    let motion = app.tree.motion.as_ref().filter(|m| m.parent < rows.len());
    let block = motion.map(TreeMotion::block);
    let removed: &[Entry] = motion.map_or(&[], |m| m.removed.as_slice());
    let reveal = motion.map_or(1.0, TreeMotion::reveal);
    let hidden = motion.map_or(0.0, TreeMotion::hidden);
    let total = rows.len() + removed.len() + usize::from(inserted.is_some());

    // 内容高度与点击、拖动滚动条使用的相同（见 `tree_max_scroll`：末尾多留一行），动画中减去
    // 尚未露出的高度。行数变少之后（收起目录、外部删除了文件），滚动位置在这里夹回有效范围；
    // 收起动画中，滚动位置随露出的高度平滑回收。
    let content = (total + 1) as f32 * TREE_ROW - hidden;
    let scroll = app.tree.scroll.clamp(0.0, (content - tree.h).max(0.0));
    // 固定在顶部的目录按该滚动位置计算，因此先写回 `app.tree.scroll`。
    app.tree.scroll = scroll;

    // 返回按展开后的行序数的第 `v` 行对应的条目，以及该条目在文件树数据中的行号；正在收起的子行
    // 已不在文件树数据中，行号为 `None`。
    let entry_at = |v: usize| -> (&Entry, Option<usize>) {
        match block {
            Some((start, end)) if !removed.is_empty() && v >= start => {
                if v < end {
                    (&removed[v - start], None)
                } else {
                    (&rows[v - removed.len()], Some(v - removed.len()))
                }
            }
            _ => (&rows[v], Some(v)),
        }
    };

    // 拖动时（也包括从系统文件管理器拖着文件经过），作为放置目标的目录连同其已展开的子行铺一层
    // 底色；目标是根目录时覆盖整棵树。拖动期间的展开只会是停在目录上触发的自动展开，不存在正在
    // 收起的子行，因此 `removed` 总是空的。
    let target = match app.file_hover() {
        Some(DropZone::Tree(dir)) => Some(dir),
        _ => app.tree_dragging().and_then(|d| d.target.as_ref()),
    };
    if let Some(dir) = target.filter(|_| removed.is_empty()) {
        let area = if *dir == ws.root {
            Some(tree.inset(8.0, 2.0))
        } else {
            // 合并显示的一行上，放进哪一级就高亮哪一级（见 `draw_chain`），这里只铺最后一级的子项。
            ws.row_of(dir)
                .filter(|&start| rows[start].path == *dir)
                .and_then(|start| {
                    let count = rows[start].end - start - 1;
                    let merged = !rows[start].chain().is_empty();
                    if merged && count == 0 {
                        return None;
                    }
                    let first = start + usize::from(merged);
                    let (top, _) = tree_row_slot(tree, scroll, first, block, hidden);
                    let (last, clip) = tree_row_slot(tree, scroll, start + count, block, hidden);
                    let bottom = last.bottom().min(clip.bottom());
                    Some(Rect::new(
                        tree.x + 8.0,
                        top.y + 1.0,
                        tree.w - 16.0,
                        bottom - top.y - 2.0,
                    ))
                })
        };
        if let Some(area) = area {
            frame.rect_clipped(area, p.drop_target, RADIUS_ROW, tree);
        }
    }

    let first = (scroll / TREE_ROW).floor().max(0.0) as usize;
    let visible = (tree.h / TREE_ROW).ceil() as usize + 1 + (hidden / TREE_ROW).ceil() as usize;
    // 横向滚动使缩进、图标和名称一起左移；底色的圆角矩形不动。
    let shift = -app.tree.scroll_x;
    let mut segments = Vec::new();
    let mut widest: f32 = 0.0;

    for v in first..(first + visible).min(total) {
        let (row, row_clip) = tree_row_slot(tree, scroll, v, block, hidden);

        // 输入行（重命名时是原条目所在的行）改为绘制输入框，不绘制条目。
        if editing == Some(v) {
            if let Some(entry) = name_input.as_mut() {
                draw_name_input(app, frame, entry, row, row_clip, shift, &mut ime);
            }

            continue;
        }

        // 输入行占用一个显示槽位，其后的条目在数据中的行号比显示槽位号小 1，位置不变。
        let (entry, index) = entry_at(match inserted {
            Some(at) if v > at => v - 1,
            _ => v,
        });

        if row.bottom() <= row_clip.y || row.y >= row_clip.bottom() {
            continue;
        }

        // 正在展开或收起的目录，箭头随动画进度转动。
        let open = match motion {
            Some(m) if index == Some(m.parent) => reveal,
            _ if entry.expanded => 1.0,
            _ => 0.0,
        };
        let parts = (row, row_clip, open);
        let width = draw_tree_entry(app, frame, entry, index, parts, &mut segments);
        widest = widest.max(width);
    }

    draw_sticky(app, frame, rows, &mut segments);
    app.hits.tree_segments = segments;
    app.tree.labels.end_frame();

    draw_scrollbar(app, frame, ScrollArea::Tree, 0, tree, scroll, content);

    // 最宽的一行决定横向能滚多远，只统计当前屏幕上的可见行。末尾留出 20 逻辑像素，使滚到头时
    // 名称不贴着侧栏边缘。收起目录、刷新之后该宽度会变小，横向滚动位置随之回收。
    app.tree.content_w = widest + 20.0;
    app.clamp_tree_scroll_x();
    app.name_input = name_input;
    app.ime.cursor = ime;

    let thumb = hscrollbar_thumb(tree, app.tree.scroll_x, app.tree.content_w);
    let look = app.scrollbar_look(
        ScrollArea::TreeHorizontal,
        0,
        app.tree.scroll_x,
        thumb.map(|_| tree),
    );
    if let Some(thumb) = thumb {
        draw_thumb(frame, p, thumb, look);
    }
}

/// 绘制文件树的一行：选中或悬停的底色、缩进参考线、箭头、图标和名称。
///
/// `open` 是目录箭头已转过的比例（0 为收起，1 为展开）。返回从行左边缘到名称末尾的宽度（逻辑像素，
/// 不含横向滚动偏移），横向滚动范围据此计算。
fn draw_tree_entry(
    app: &App,
    frame: &mut Frame<'_>,
    entry: &Entry,
    index: Option<usize>,
    (row, clip, open): (Rect, Rect, f32),
    segments: &mut Vec<(usize, f32, f32, PathBuf)>,
) -> f32 {
    let p = app.palette;
    let tree = app.layout.tree;
    let shift = -app.tree.scroll_x;
    let ws = app.editor.workspace.as_ref();
    let selected = index.is_some_and(|i| ws.is_some_and(|w| w.is_selected(i)));
    let hovered = index.is_some_and(|i| {
        app.hover == Hover::TreeRow(i) || app.tree.overflow.is_some_and(|(_, o)| o == i)
    });

    // 名称放不下时，底色一直铺到侧栏边缘、右侧不留圆角，看起来像被侧栏边缘截断；
    // 鼠标移上去时在编辑区上方继续绘制完整名称（见 `draw_tree_overflow`）。
    let label_x = tree_label_x(row, entry.depth, shift);
    let label = entry.label();
    // 合并显示的一行放不下时折叠中间的各级；鼠标在这一行上时展开完整路径，任意一级都点得到。
    let room = if app.git.change_of(&entry.path).is_some() {
        tree.right() - 16.0 - GIT_DOT - 6.0
    } else {
        tree.right() - 12.0
    } - label_x;
    let folded = chain_folded(app, frame, entry, room);
    let full = label_width(app, frame, entry);
    // 悬停和收回时名字的内容、宽度都缓动；放不下的普通名字收回到侧栏边缘。
    let shown: Vec<Cow<'_, str>> = match &folded {
        Some((parts, _)) if !hovered => parts.iter().map(|&part| part_name(entry, part)).collect(),
        Some(_) => entry
            .chain()
            .iter()
            .map(|p| winc_editor::segment_name(p))
            .collect(),
        None => vec![Cow::Borrowed(entry.name.as_str())],
    };
    let target = match &folded {
        Some((_, width)) if !hovered => *width,
        _ if hovered => full,
        _ => full.min(room.max(0.0)),
    };
    let cause = LabelCause {
        hovered,
        levels: entry.chain().len(),
        current: chain_current(app, entry),
    };
    let view = app
        .tree
        .labels
        .sync(label_key(entry), cause, &shown, target);
    let label_end = match &folded {
        None if !hovered => label_x + full,
        _ => label_x + view.width,
    };
    let overflows = label_overflows(tree, label_end);
    let pill = tree_row_pill(row, overflows);

    let focused = app.focus == Focus::Tree;
    let retracting = !hovered && view.progress < 1.0 && label_x + view.width > tree.right() - 12.0;
    let background = if retracting {
        retracting_color(p, selected, focused, view.progress)
    } else {
        row_color(p, selected, focused, hovered)
    };
    if let Some(color) = background {
        frame.rect_corners(pill.0, color, pill.1, clip);
    }

    let indent = row.x + 14.0 + shift;
    let cy = icon_center_y(app, row, app.fonts.ui);

    // 缩进参考线：每级一条，位于父目录箭头的正下方，相邻行的参考线连成竖线。
    for level in 0..entry.depth {
        let x = (indent + level as f32 * TREE_INDENT + 4.0).round();
        frame.rect_clipped(Rect::new(x, row.y, 1.0, row.h), p.indent_guide, 0.0, clip);
    }

    let mut x = indent + entry.depth as f32 * TREE_INDENT;

    if entry.kind == EntryKind::Dir {
        let angle = open * FRAC_PI_2;
        icons::chevron_turned(frame, (x + 4.0, cy), 6.0, angle, p.text_dim, clip);
    }

    x += 14.0;

    if entry.kind == EntryKind::Dir {
        let top = (cy - 6.0).round();
        icons::folder(frame, (x, top), entry.expanded, p.folder, clip);
    } else {
        draw_file_icon(frame, p, (x + 1.5, cy), &entry.name, clip);
    }

    // 右端的 Git 状态与提交视图相同：文件显示带颜色的字母（M、U、A……），目录中有改动时
    // 显示圆点。名称会被状态截断的行在悬停时不绘制状态，以完整显示名称；
    // 放不下的部分继续绘制在编辑区上方。
    let change = app.git.change_of(&entry.path);
    let letter = change
        .filter(|_| entry.kind != EntryKind::Dir)
        .map(|c| c.letter().to_string());
    let mark_w = match (&letter, change) {
        (Some(letter), _) => frame.measure(FontKind::Ui, app.fonts.ui_small, letter),
        (None, Some(_)) => GIT_DOT,
        (None, None) => 0.0,
    };
    let mark_x = tree.right() - 16.0 - mark_w;
    let change = change.filter(|_| !(hovered && label_end > mark_x - 6.0));
    // 名字画到哪里为止：有状态时停在状态左边，没有时到侧栏边缘。
    let (text_right, cut) = match change {
        Some(_) => (mark_x - 6.0, label_end > mark_x - 6.0),
        None => (tree.right(), overflows),
    };

    let color = tree_entry_color(app, entry, selected);
    let text_clip = Rect::new(clip.x, clip.y, (text_right - clip.x).max(0.0), clip.h);
    let text_clip = text_clip.intersect(clip);
    // 内容变化途中，名字只露出到当前宽度。
    let text_clip = if view.old.is_empty() {
        text_clip
    } else {
        let w = (label_x + view.width + CHAIN_GAP - text_clip.x).max(0.0);
        Rect::new(text_clip.x, text_clip.y, w.min(text_clip.w), text_clip.h)
    };
    draw_old_label(app, frame, (label_x, row), &view, color, text_clip);
    if entry.chain().is_empty() {
        let shown = if view.common > 0 {
            color
        } else {
            color.alpha(color.a * view.alpha)
        };
        ui_text(frame, app, label_x, row, &label, shown, text_clip);
    } else {
        let style = (color, selected);
        let parts = folded
            .as_ref()
            .filter(|_| !hovered)
            .map(|(parts, _)| parts.as_slice());
        draw_chain(
            app,
            frame,
            entry,
            (label_x, row, parts, Some(&view)),
            style,
            text_clip,
            |left, right, path| {
                if let Some(index) = index {
                    segments.push((index, left, right, path.to_path_buf()));
                }
            },
        );
    }

    // 截断的名字逐渐淡出，避免在半个字处直接截断。悬停时不淡：完整名称继续绘制在编辑区上方
    // （[`draw_tree_overflow`]），淡出会使两段接不上。
    // 收回途中名字还连着编辑区上方那一段，到最后才淡出，名字不会先变暗、再缩回来。
    let reach = NAME_FADE * stage(view.progress, FADE_FROM, 1.0);
    if cut && !hovered && reach > 0.0 {
        let solid = background.map_or(p.sidebar_bg, |c| panel::over(p.sidebar_bg, c));
        let edge = Rect::new(text_right - reach, row.y, reach, row.h);
        frame.within(clip, |frame| fade(frame, edge, solid, false));
    }

    if let Some(change) = change {
        let color = git_color(p, change);
        match &letter {
            Some(letter) => {
                ui_text_small(frame, app, mark_x, row, letter, color, clip);
            }
            None => {
                let dot = Rect::new(mark_x, cy - GIT_DOT / 2.0, GIT_DOT, GIT_DOT);
                frame.rect_clipped(dot, color, GIT_DOT / 2.0, clip);
            }
        }
    }

    // 横向滚动范围按折叠后的宽度计算，鼠标移到这一行上时滚动条不变。
    let end = folded.map_or(label_x + full, |(_, width)| label_x + width);
    end - shift - row.x
}

/// Git 状态在文件树、变更视图里用的颜色。
pub(in crate::view) fn git_color(p: &Palette, change: Change) -> Color {
    match change {
        Change::Modified | Change::Renamed => p.git_modified,
        Change::Added | Change::Untracked => p.git_added,
        Change::Deleted => p.git_deleted,
        Change::Conflicted => p.git_conflict,
    }
}

/// 绘制固定在文件树顶部的各级目录：覆盖在滚到上方的行之上，下沿一道淡阴影把它们与滚动的行分开。
///
/// 被推上去的那一层压在上一层之下，因此按从内层到外层的顺序绘制。
fn draw_sticky(
    app: &App,
    frame: &mut Frame<'_>,
    rows: &[Entry],
    segments: &mut Vec<(usize, f32, f32, PathBuf)>,
) {
    let sticky = app.tree_sticky();
    let Some(bottom) = sticky.iter().map(|s| s.rect.bottom()).reduce(f32::max) else {
        return;
    };
    let p = app.palette;
    let tree = app.layout.tree;

    frame.within(tree, |frame| {
        let header = Rect::new(tree.x, tree.y, tree.w, bottom - tree.y);
        let covered = frame.covered(header, 0.0);
        frame.shadow(header, 0.0, 6.0, 0.0, p.shadow, Some(covered));

        for s in sticky.iter().rev() {
            frame.rect_clipped(s.rect, p.sidebar_bg, 0.0, tree);
            draw_tree_entry(
                app,
                frame,
                &rows[s.index],
                Some(s.index),
                (s.rect, tree, 1.0),
                segments,
            );
        }
    });
}

/// 绘制文件树中正在输入名称的那一行：图标、输入框，出错时右侧显示原因；光标矩形写入 `ime`。
///
/// 位置与普通行一致（缩进、图标都在相同的位置），使该行看起来就是目录中即将出现的一项。
fn draw_name_input(
    app: &App,
    frame: &mut Frame<'_>,
    entry: &mut NameInput,
    row: Rect,
    row_clip: Rect,
    shift: f32,
    ime: &mut Option<Rect>,
) {
    let p = app.palette;
    let depth = entry.depth;
    let kind = entry.kind;
    let error = entry.error.clone();
    let cy = icon_center_y(app, row, app.fonts.ui);
    // 正在输入的这一行铺一层圆角底色（与其他行的底色一样大），可以看出在编辑哪一行；出错时
    // 底色外加一圈红色细线，出错原因写在底色内右侧，与红线连在一起。不另绘制一个输入框：树里
    // 一行很窄，再套一个框显得拥挤。
    let text_x = tree_label_x(row, depth, shift);
    let focused = app.focus == Focus::Prompt;
    let (pill, radii) = tree_row_pill(row, false);
    if error.is_some() {
        let ring = radii.map(|r| r + 1.0);
        frame.rect_corners(pill.inset(-1.0, -1.0), p.error, ring, row_clip);
    }
    // 用悬停的浅色而不是选中色：重命名时文件名主干是选中的，选中色底上看不出文字的选区。
    // 先和侧栏底色混合成不透明的颜色，盖住底下的红线。
    let fill = super::panel::over(p.sidebar_bg, p.hover);
    frame.rect_corners(pill, fill, radii, row_clip);

    // 缩进和图标按普通行的位置放置，与上下相邻的条目对齐。
    let indent = row.x + 14.0 + shift;
    let x = indent + depth as f32 * TREE_INDENT + 14.0;

    // 缩进参考线：与普通行相同，每级一条。
    for level in 0..depth {
        let guide = (indent + level as f32 * TREE_INDENT + 4.0).round();
        frame.rect_clipped(
            Rect::new(guide, row.y, 1.0, row.h),
            p.indent_guide,
            0.0,
            row_clip,
        );
    }

    // 重命名目录时仍然绘制箭头，使该行看起来就是原来的那一行变成了输入框。
    let open = match entry.rename {
        Some((_, expanded)) => {
            if kind == EntryKind::Dir {
                let angle = if expanded { FRAC_PI_2 } else { 0.0 };
                let arrow = (x - 14.0 + 4.0, cy);
                icons::chevron_turned(frame, arrow, 6.0, angle, p.text_dim, row_clip);
            }
            expanded
        }
        None => true,
    };

    if kind == EntryKind::Dir {
        icons::folder(frame, (x, (cy - 6.0).round()), open, p.folder, row_clip);
    } else {
        draw_file_icon(frame, p, (x + 1.5, cy), entry.field.text(), row_clip);
    }

    // 出错时底色里右侧留出显示原因的位置，文字只使用剩余宽度。
    let error_room = error.as_ref().map_or(0.0, |e| {
        frame.measure(FontKind::Ui, app.fonts.ui_small, e) + 16.0
    });
    let width = (pill.right() - 4.0 - error_room - text_x).max(0.0);
    let area = Rect::new(text_x, row.y, width, row.h);
    let caret = draw_bar_field(
        app,
        frame,
        area,
        &mut entry.field,
        if entry.rename.is_some() {
            t!("workspace-tree-rename-placeholder")
        } else {
            t!("workspace-tree-create-placeholder")
        },
        focused,
    );

    if let Some(error) = &error {
        let x = pill.right() - 8.0 - frame.measure(FontKind::Ui, app.fonts.ui_small, error);
        ui_text_small(frame, app, x, row, error, p.error, row_clip);
    }

    *ime = caret;
}

/// 名字收回时，末尾的淡出从过渡的这个进度开始出现（经验值）：名字已基本缩回侧栏。
pub(in crate::view) const FADE_FROM: f32 = 0.7;
/// 合并显示的目录中没有突出的各级的不透明度（经验值）：比突出的那一级淡，仍能看清。
const CHAIN_DIM: f32 = 0.6;
/// 合并显示的目录之间分隔符的不透明度（经验值）：比目录名淡，一眼能分出各级。
pub(in crate::view) const CHAIN_SEPARATOR: f32 = 0.45;
/// 合并显示的目录之间分隔符两侧各留的空白（逻辑像素，经验值）。
pub(in crate::view) const CHAIN_GAP: f32 = 4.0;
/// 合并显示的目录放不下时代替被折叠的各级的记号。
pub(in crate::view) const CHAIN_FOLD: &str = "…";

/// 返回文件树一行内名称起点的 x 坐标：位于缩进、箭头和图标之后，再加上横向滚动偏移
/// `shift`（为负值）。
///
/// 目录和文件的图标同宽。
fn tree_label_x(row: Rect, depth: usize, shift: f32) -> f32 {
    row.x + 48.0 + depth as f32 * TREE_INDENT + shift
}

/// 是否名称末尾 `label_end` 超出了该行圆角底色的右边界。
fn label_overflows(tree: Rect, label_end: f32) -> bool {
    label_end > tree.right() - 12.0
}

/// 返回一行的圆角底色矩形和四个角的半径。
///
/// 名称放不下（`overflows`）时底色铺到侧栏边缘，右侧为直角。
fn tree_row_pill(row: Rect, overflows: bool) -> (Rect, [f32; 4]) {
    let (y, h) = (row.y + 1.0, row.h - 2.0);

    if overflows {
        let r = RADIUS_ROW;
        (Rect::new(row.x + 8.0, y, row.w - 8.0, h), [r, 0.0, 0.0, r])
    } else {
        (Rect::new(row.x + 8.0, y, row.w - 16.0, h), [RADIUS_ROW; 4])
    }
}

/// 返回文件树中 `entry` 名称的颜色：编辑区中当前打开的文件用强调色标出（已选中时除外）；
/// 其余有 Git 状态的用状态色，被 Git 忽略的用淡色，都没有的用正文色。
///
/// 按住另一个文件、尚未松开时，当前打开的文件不用强调色：选中在按下时移走，文件在松开时才
/// 打开，其间若改用强调色，该文件会在按下与松开之间多一次颜色变化。
///
/// 文件树中的行与放不下时在编辑区上方继续绘制的部分共用该颜色，使两段颜色连续。
fn tree_entry_color(app: &App, entry: &Entry, selected: bool) -> Color {
    let p = &app.palette;
    let active = app.editor.active().and_then(|d| d.path.as_deref());
    let is_active = !app.tree_open_pending() && active == Some(entry.path.as_path());
    let color = match app.git.change_of(&entry.path) {
        Some(change) if !is_active || selected => git_color(p, change),
        _ if is_active && !selected => p.accent,
        // 被 Git 忽略的条目（`target/`、`node_modules/` 及其中的项）用淡色绘制，
        // 表示其不受版本控制。
        _ if app.git.is_ignored(&entry.path) => p.text_dim,
        _ => p.text,
    };
    // 剪切了、还没粘贴的项绘制得更淡；合并显示的目录逐级判断（见 `draw_chain`）。
    if entry.chain().is_empty() && app.shared.cut_files.borrow().contains(&entry.path) {
        color.alpha(color.a * 0.5)
    } else {
        color
    }
}

/// 返回鼠标所在的文件树行在名称放不下时，于编辑区上方继续绘制的那一段：矩形和该行的行号。
///
/// 不需要接续绘制时返回 `None`。
pub(in crate::view) fn tree_overflow(app: &App, frame: &mut Frame<'_>) -> Option<(Rect, usize)> {
    let index = app.overflow_candidate()?;
    let entry = app.editor.workspace.as_ref()?.rows().get(index)?;
    let tree = app.layout.tree;
    // 固定在顶部的目录在固定的位置继续绘制。
    let row = match app.tree_sticky().into_iter().find(|s| s.index == index) {
        Some(sticky) => sticky.rect,
        None => tree_row_slot(tree, app.tree.scroll, app.tree_slot_of(index), None, 0.0).0,
    };
    let shift = -app.tree.scroll_x;
    let end = tree_label_x(row, entry.depth, shift) + label_width(app, frame, entry);

    if !label_overflows(tree, end) {
        return None;
    }

    let right = (end + 8.0).min(app.layout.window.right() - 8.0);
    let rect = Rect::new(tree.right(), row.y + 1.0, right - tree.right(), row.h - 2.0);
    (rect.w > 0.0 && app.mouse().0 < rect.right()).then_some((rect, index))
}

/// 在编辑区上方继续绘制放不下的文件名：底色和文本都与文件树中的那一行连成一体。
pub(in crate::view) fn draw_tree_overflow(app: &App, frame: &mut Frame<'_>) {
    let (Some((rect, index)), Some(ws)) = (app.tree.overflow, &app.editor.workspace) else {
        return;
    };
    let Some(entry) = ws.rows().get(index) else {
        return;
    };
    let view = app.tree.labels.peek(label_key(entry));
    draw_overflow_band(app, frame, (entry, index), rect, view.as_ref(), false);
}

/// 绘制鼠标移开后正在收回的名字：不响应点击，收回到侧栏边缘时消失。
pub(in crate::view) fn draw_tree_overflow_closing(app: &App, frame: &mut Frame<'_>) {
    // 文件树不在显示时不画：动画途中隐藏了侧栏或切换到其他视图。
    if !app.sidebar_open() || app.sidebar.view != SidebarView::Files {
        return;
    }
    let Some(ws) = &app.editor.workspace else {
        return;
    };
    let open = app.tree.overflow.map(|(_, index)| index);
    let tree = app.layout.tree;
    for (key, view) in app.tree.labels.moving_rows() {
        let Some(index) = ws.row_of(&key).filter(|&index| Some(index) != open) else {
            continue;
        };
        let entry = &ws.rows()[index];
        let row = overflow_row(app, index);
        let end = tree_label_x(row, entry.depth, -app.tree.scroll_x) + view.width;
        let right = (end + 8.0).min(app.layout.window.right() - 8.0);
        let rect = Rect::new(tree.right(), row.y + 1.0, right - tree.right(), row.h - 2.0);
        if rect.w > 0.0 {
            draw_overflow_band(app, frame, (entry, index), rect, Some(&view), true);
        }
    }
}

/// 返回第 `index` 行在文件树中的矩形；固定在顶部的目录在固定的位置。
fn overflow_row(app: &App, index: usize) -> Rect {
    let tree = app.layout.tree;
    match app.tree_sticky().into_iter().find(|s| s.index == index) {
        Some(sticky) => sticky.rect,
        None => tree_row_slot(tree, app.tree.scroll, app.tree_slot_of(index), None, 0.0).0,
    }
}

/// 在 `rect` 中绘制第 `index` 行超出侧栏的部分，按 `view` 的当前宽度露出。
///
/// `closing` 为真时显示收回途中的旧内容，否则显示这一行的完整名字。
fn draw_overflow_band(
    app: &App,
    frame: &mut Frame<'_>,
    (entry, index): (&Entry, usize),
    rect: Rect,
    view: Option<&LabelView>,
    closing: bool,
) {
    let Some(ws) = &app.editor.workspace else {
        return;
    };
    let p = app.palette;
    let tree = app.layout.tree;
    let row = Rect::new(tree.x, rect.y - 1.0, tree.w, rect.h + 2.0);
    let x = tree_label_x(row, entry.depth, -app.tree.scroll_x);
    // 命中范围按完整宽度计算，绘制时才按当前宽度露出：过渡途中鼠标移过去不会落空。
    let right = view.map_or(rect.right(), |v| rect.right().min(x + v.width + 8.0));
    if right <= rect.x {
        return;
    }
    // 这一行在最上层：底色从滚动条的范围起，滚动条只被这一行的高度挡住，上下两段照常显示。
    let left = tree.right() - SCROLLBAR_HIT;
    let rect = Rect::new(left, rect.y, right - left, rect.h);
    let clip = Rect::new(rect.x, tree.y, rect.w, tree.h);
    let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
    let selected = ws.is_selected(index);
    let focused = app.focus == Focus::Tree;

    // 底层先垫一层侧栏底色，再叠半透明的选中色，使颜色与文件树中的那一段一致。
    frame.rect_corners(rect, p.sidebar_bg, radii, clip);

    // 收回时与侧栏内的那一段用同一个底色，一起淡出，两段不割裂。
    let tint = match view.filter(|_| closing) {
        Some(view) => retracting_color(p, selected, focused, view.progress),
        None => row_color(p, selected, focused, true),
    };
    if let Some(color) = tint {
        frame.rect_corners(rect, color, radii, clip);
    }

    let color = tree_entry_color(app, entry, selected);
    if let Some(view) = view.filter(|view| closing && !view.old.is_empty()) {
        // 收回中的内容：相同的前缀不淡，其余随过渡淡出。
        let step = chain_step(app, frame);
        let separator = color.alpha(color.a * CHAIN_SEPARATOR);
        let mut at = x;
        for (k, name) in view.old.iter().enumerate() {
            let alpha = if k < view.common { 1.0 } else { view.old_alpha };
            let width = frame.measure(FontKind::Ui, app.fonts.ui, name);
            ui_text(
                frame,
                app,
                at,
                row,
                name,
                color.alpha(color.a * alpha),
                clip,
            );
            if k + 1 < view.old.len() {
                let bar = separator.alpha(separator.a * alpha);
                ui_text(frame, app, at + width + CHAIN_GAP, row, "/", bar, clip);
            }
            at += width + step;
        }
        return;
    }
    if let Some(view) = view {
        draw_old_label(app, frame, (x, row), view, color, clip);
    }
    if entry.chain().is_empty() {
        ui_text(frame, app, x, row, &entry.label(), color, clip);
    } else {
        draw_chain(
            app,
            frame,
            entry,
            (x, row, None, view),
            (color, selected),
            clip,
            |_, _, _| {},
        );
    }
}

/// 返回名字收回途中这一行的底色：选中的行不变，其余是悬停的底色随进度 `progress` 淡出。
pub(in crate::view) fn retracting_color(
    p: &Palette,
    selected: bool,
    focused: bool,
    progress: f32,
) -> Option<Color> {
    row_color(p, selected, focused, false).or_else(|| {
        row_color(p, selected, focused, true).map(|color| color.alpha(color.a * (1.0 - progress)))
    })
}

/// 返回当前的一级在合并显示的目录中的下标：选中了其中某一级时是那一级，否则是最后一级；没有
/// 合并时为 0。
fn chain_current(app: &App, entry: &Entry) -> usize {
    let chain = entry.chain();
    let segment = app.editor.workspace.as_ref().and_then(|w| w.segment());
    chain
        .iter()
        .position(|path| Some(path.as_path()) == segment)
        .unwrap_or(chain.len().saturating_sub(1))
}

/// 返回这一行过渡状态的键：合并显示时是最上一级，展开、收起前后不变。
fn label_key(entry: &Entry) -> &Path {
    entry.chain().first().unwrap_or(&entry.path)
}

/// 返回折叠后一部分的显示名。
fn part_name<'a>(entry: &'a Entry, part: Option<usize>) -> Cow<'a, str> {
    part.map_or(Cow::Borrowed(CHAIN_FOLD), |i| {
        winc_editor::segment_name(&entry.chain()[i])
    })
}

/// 返回相邻两部分的间距：分隔符和两侧的留白。
fn chain_step(app: &App, frame: &mut Frame<'_>) -> f32 {
    frame.measure(FontKind::Ui, app.fonts.ui, "/") + CHAIN_GAP * 2.0
}

/// 绘制过渡中淡出的旧内容，相同的前缀由新内容绘制，这里跳过。
fn draw_old_label(
    app: &App,
    frame: &mut Frame<'_>,
    (start, row): (f32, Rect),
    view: &LabelView,
    color: Color,
    clip: Rect,
) {
    if view.old.is_empty() {
        return;
    }
    let step = chain_step(app, frame);
    let text = color.alpha(color.a * view.old_alpha);
    let separator = color.alpha(color.a * CHAIN_SEPARATOR * view.old_alpha);
    let mut x = start;
    for (k, name) in view.old.iter().enumerate() {
        let width = frame.measure(FontKind::Ui, app.fonts.ui, name);
        if k >= view.common {
            ui_text(frame, app, x, row, name, text, clip);
        }
        if k + 1 < view.old.len() && k + 1 > view.common {
            ui_text(frame, app, x + width + CHAIN_GAP, row, "/", separator, clip);
        }
        x += width + step;
    }
}

/// 返回这一行名称的宽度（逻辑像素）；合并显示的目录含各级之间的分隔符和留白。
fn label_width(app: &App, frame: &mut Frame<'_>, entry: &Entry) -> f32 {
    let size = app.fonts.ui;
    let chain = entry.chain();
    if chain.is_empty() {
        return frame.measure(FontKind::Ui, size, &entry.name);
    }
    let names: f32 = chain
        .iter()
        .map(|path| frame.measure(FontKind::Ui, size, &winc_editor::segment_name(path)))
        .sum();
    let slash = frame.measure(FontKind::Ui, size, "/") + CHAIN_GAP * 2.0;
    names + slash * (chain.len() - 1) as f32
}

/// 逐级绘制合并显示的目录，起点是 `at` 的横坐标和所在的行；`report` 收到每一级的左右边界和路径。
///
/// 拖动放进其中某一级或鼠标悬停在某一级时，那一级加上底色并突出；选中这一行（`style` 的第二项）
/// 时突出选中的那一级。突出的一级用文字颜色（`style` 的第一项），其余按 [`CHAIN_DIM`] 变淡，被 Git
/// 忽略等本身较淡的文字也能分出是哪一级。分隔符和留白归入相邻的两级，各占一半。
///
/// `at` 的第三项是要显示的各部分（见 [`fold_chain`]），`None` 时显示全部；第四项是过渡的绘制
/// 参数，相同的前缀之后的部分按它淡入。
fn draw_chain(
    app: &App,
    frame: &mut Frame<'_>,
    entry: &Entry,
    (start, row, parts, view): (f32, Rect, Option<&[Option<usize>]>, Option<&LabelView>),
    (color, selected): (Color, bool),
    clip: Rect,
    mut report: impl FnMut(f32, f32, &Path),
) {
    let ws = app.editor.workspace.as_ref();
    let chain = entry.chain();
    let dropping = app
        .tree_dragging()
        .and_then(|d| d.target.as_deref())
        .filter(|target| chain.iter().any(|path| path == target));
    let current = dropping
        .or_else(|| ws.and_then(|w| w.segment()))
        .unwrap_or(&entry.path);
    let emphasize = dropping.is_some() || selected && ws.is_some_and(|w| !w.multi_selected());
    let faded = color.alpha(color.a * CHAIN_DIM);
    let separator = color.alpha(color.a * CHAIN_SEPARATOR);
    let slash = frame.measure(FontKind::Ui, app.fonts.ui, "/");
    let step = slash + CHAIN_GAP * 2.0;
    let every: Vec<Option<usize>>;
    let parts = match parts {
        Some(parts) => parts,
        None => {
            every = (0..chain.len()).map(Some).collect();
            &every
        }
    };
    let mut x = start;
    for (k, part) in parts.iter().enumerate() {
        let fade = view
            .filter(|view| k >= view.common)
            .map_or(1.0, |view| view.alpha);
        let width = match part {
            Some(i) => {
                let path = &chain[*i];
                let name = winc_editor::segment_name(path);
                let width = frame.measure(FontKind::Ui, app.fonts.ui, &name);
                let hovered = dropping.is_none()
                    && matches!(app.hover, Hover::TreeRow(_))
                    && app.tree.hover_segment.as_ref() == Some(path);
                if hovered || dropping == Some(path.as_path()) {
                    let area = Rect::new(
                        x - CHAIN_GAP,
                        row.y + 3.0,
                        width + CHAIN_GAP * 2.0,
                        row.h - 6.0,
                    );
                    frame.rect_clipped(area, app.palette.drop_target, RADIUS_ROW, clip);
                }
                let text = if emphasize && path != current && !hovered {
                    faded
                } else {
                    color
                };
                let text = if app.shared.cut_files.borrow().contains(path) {
                    text.alpha(text.a * 0.5)
                } else {
                    text
                };
                let text = text.alpha(text.a * fade);
                ui_text(frame, app, x, row, &name, text, clip);
                let left = if k == 0 { x } else { x - step / 2.0 };
                report(left, x + width + step / 2.0, path);
                width
            }
            None => {
                let mark = separator.alpha(separator.a * fade);
                ui_text(frame, app, x, row, CHAIN_FOLD, mark, clip);
                frame.measure(FontKind::Ui, app.fonts.ui, CHAIN_FOLD)
            }
        };
        if k + 1 < parts.len() {
            let bar = separator.alpha(separator.a * fade);
            ui_text(frame, app, x + width + CHAIN_GAP, row, "/", bar, clip);
        }
        x += width + step;
    }
}

/// 返回这一行折叠后要显示的各部分及其宽度（逻辑像素），放得进 `room` 时不折叠；不是合并显示的
/// 一行时返回 `None`。
fn chain_folded(
    app: &App,
    frame: &mut Frame<'_>,
    entry: &Entry,
    room: f32,
) -> Option<(Vec<Option<usize>>, f32)> {
    let chain = entry.chain();
    if chain.is_empty() {
        return None;
    }
    let size = app.fonts.ui;
    let widths: Vec<f32> = chain
        .iter()
        .map(|path| frame.measure(FontKind::Ui, size, &winc_editor::segment_name(path)))
        .collect();
    let step = frame.measure(FontKind::Ui, size, "/") + CHAIN_GAP * 2.0;
    let fold = frame.measure(FontKind::Ui, size, CHAIN_FOLD);
    Some(fold_chain(
        &widths,
        step,
        fold,
        chain_current(app, entry),
        room,
    ))
}

/// 折叠放不进 `room` 的目录链，返回从左到右要显示的各部分（`Some` 是目录的下标，`None` 是一处折叠）
/// 和总宽度。
///
/// `widths` 是各级名字的宽度，`step` 是相邻两部分之间分隔符和留白的宽度，`fold` 是折叠记号的宽度。
/// 第一级和当前的一级 `current` 始终显示，最后一级放得下就显示；还有空间时从离 `current` 近的
/// 各级依次补上，距离相同时先补靠后的。
pub(in crate::view) fn fold_chain(
    widths: &[f32],
    step: f32,
    fold: f32,
    current: usize,
    room: f32,
) -> (Vec<Option<usize>>, f32) {
    let last = widths.len() - 1;
    let lay_out = |keep: &[bool]| {
        let mut parts = Vec::new();
        for (i, &kept) in keep.iter().enumerate() {
            if kept {
                parts.push(Some(i));
            } else if parts.last() != Some(&None) {
                parts.push(None);
            }
        }
        let width: f32 = parts
            .iter()
            .map(|part| part.map_or(fold, |i| widths[i]))
            .sum();
        let width = width + step * parts.len().saturating_sub(1) as f32;
        (parts, width)
    };

    let all = lay_out(&vec![true; widths.len()]);
    if all.1 <= room {
        return all;
    }
    let mut keep = vec![false; widths.len()];
    keep[0] = true;
    keep[current] = true;
    keep[last] = true;
    if lay_out(&keep).1 > room && current != last {
        keep[last] = false;
    }
    let mut order: Vec<usize> = (0..widths.len()).collect();
    order.sort_by_key(|&i| (i.abs_diff(current), std::cmp::Reverse(i)));
    for i in order {
        if keep[i] {
            continue;
        }
        keep[i] = true;
        if lay_out(&keep).1 > room {
            keep[i] = false;
            break;
        }
    }
    lay_out(&keep)
}

/// 绘制侧栏开关按钮；侧栏展开时图标显示为展开状态。
pub(in crate::view) fn draw_sidebar_button(app: &App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let button = app.layout.sidebar_button;
    let open = app.sidebar_open();

    let hovered = app.hover == Hover::SidebarButton;
    draw_button(frame, p, button, app.layout.window, hovered, false);
    icons::sidebar(frame, center(button), open, p.text_muted, button);
}

#[cfg(test)]
mod tests;
