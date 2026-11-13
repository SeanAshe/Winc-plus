//! 窗口内绘制的菜单：没有系统菜单可用时（macOS 以外的平台，以及没有窗口的测试中），
//! 右键菜单和按钮菜单在窗口里画出来，由应用自己处理悬停、点击和按键。
//!
//! 菜单项从 [`MenuEntry`] 生成，编号与系统菜单相同，选中后按编号找回命令，交给
//! [`App::run_menu_command`]；状态栏路径的目录菜单例外，各行是文件和目录
//! （[`PopupRow::Entry`]），目录停留时才读内容。

use std::path::{Path, PathBuf};

use winit::event::{KeyEvent, MouseButton};
use winit::keyboard::{Key, NamedKey};

use super::*;

/// 菜单中的一行。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PopupRow {
    /// 分隔线。
    Separator,
    /// 一条命令：文字、左边的图标、快捷键的显示文字、是否可用、是否打勾、编号（同系统菜单的
    /// tag）。
    Item {
        label: MenuLabel,
        icon: Option<MenuIcon>,
        key: String,
        enabled: bool,
        checked: bool,
        tag: isize,
    },
    /// 子菜单：标题、是否打勾、子项。
    Submenu {
        label: MenuLabel,
        checked: bool,
        rows: Vec<PopupRow>,
    },
    /// 目录菜单中的一个文件或目录，含名字、路径、是否目录、是否打勾。
    ///
    /// 打勾表示当前文件及其各级上层目录。目录停留时展开内容，文件选中时打开。
    Entry {
        name: String,
        path: PathBuf,
        dir: bool,
        checked: bool,
    },
}

impl PopupRow {
    /// 是否可以悬停和选中；分隔线和不可用的命令返回 `false`。
    pub(crate) fn selectable(&self) -> bool {
        match self {
            PopupRow::Separator => false,
            PopupRow::Item { enabled, .. } => *enabled,
            PopupRow::Submenu { .. } | PopupRow::Entry { .. } => true,
        }
    }

    /// 是否在停留时展开子菜单：子菜单行和目录行返回 `true`。
    fn expands(&self) -> bool {
        matches!(
            self,
            PopupRow::Submenu { .. } | PopupRow::Entry { dir: true, .. }
        )
    }
}

/// 打开着的一级菜单：最外层的菜单，或者展开的子菜单。
#[derive(Debug, Clone, Default)]
pub(crate) struct PopupLevel {
    /// 各行。
    pub rows: Vec<PopupRow>,
    /// 子菜单展开自哪一行（上一级菜单里的下标）；最外层为 `None`。
    pub parent: Option<usize>,
    /// 鼠标或键盘停在的那一行。
    pub hover: Option<usize>,
    /// 内容比菜单高时向下滚动的距离（逻辑像素）。
    pub scroll: f32,
    /// 绘制时记下的外框；还没有绘制过时为空。
    pub rect: Rect,
    /// 绘制时记下的各行位置（已减去滚动距离），与 `rows` 一一对应。
    pub row_rects: Vec<Rect>,
    /// 内容的总高度（逻辑像素），绘制时记下，滚动时限制范围。
    pub content: f32,
}

/// 窗口内绘制的菜单。
#[derive(Debug, Clone)]
pub(crate) struct PopupMenu {
    /// 打开着的各级菜单，第 0 级是弹出的菜单。
    pub levels: Vec<PopupLevel>,
    /// 弹出的位置：菜单的左上角；`right_aligned` 时是右上角。
    pub at: (f32, f32),
    /// 菜单向左展开（拆分按钮、标签列表的菜单）。
    pub right_aligned: bool,
    /// 打勾的那一行对准 `at`（状态栏的缩进、行尾菜单，和弹出式按钮一样）。
    pub at_checked: bool,
    /// 第一级菜单的最小宽度（逻辑像素）；没有要求时为 0。
    pub min_width: f32,
    /// 目录菜单：当前文件的路径，展开目录时据此给它的各级上层目录打勾；其他菜单为 `None`。
    pub current: Option<PathBuf>,
}

impl PopupMenu {
    /// 由第一级菜单的各行 `rows` 创建。
    ///
    /// `at` 是弹出位置，`flags` 是 `(at_checked, right_aligned)` 两个标志。
    pub(crate) fn new(rows: Vec<PopupRow>, at: (f32, f32), flags: (bool, bool)) -> PopupMenu {
        PopupMenu {
            levels: vec![PopupLevel {
                rows,
                ..PopupLevel::default()
            }],
            at,
            at_checked: flags.0,
            right_aligned: flags.1,
            min_width: 0.0,
            current: None,
        }
    }

    /// 返回点 `(x, y)` 落在第几级菜单的第几行，行号是该级 `rows` 的下标。
    ///
    /// 落在菜单的空白处（上下边距）时行号为 `None`；不在任何一级菜单上时返回 `None`。
    pub(crate) fn hit(&self, x: f32, y: f32) -> Option<(usize, Option<usize>)> {
        // 后展开的子菜单压在上面，先查。
        self.levels.iter().enumerate().rev().find_map(|(l, level)| {
            if !level.rect.contains(x, y) {
                return None;
            }
            let row = level.row_rects.iter().position(|r| {
                r.contains(x, y) && r.y >= level.rect.y && r.bottom() <= level.rect.bottom() + 0.5
            });
            Some((l, row))
        })
    }

    /// 停在第 `level` 级的第 `row` 行：更深的子菜单收起；这一行是子菜单时展开它。
    fn hover(&mut self, level: usize, row: Option<usize>) {
        self.levels.truncate(level + 1);
        let current = &mut self.levels[level];
        current.hover = row.filter(|&r| current.rows[r].selectable());
        let Some(r) = current.hover else {
            return;
        };
        let rows = match &current.rows[r] {
            PopupRow::Submenu { rows, .. } => rows.clone(),
            PopupRow::Entry {
                path, dir: true, ..
            } => folder_rows(path, self.current.as_deref().unwrap_or(path)),
            _ => return,
        };
        self.levels.push(PopupLevel {
            rows,
            parent: Some(r),
            ..PopupLevel::default()
        });
    }

    /// 在最深一级菜单中把停着的行移到上一个或下一个能选的行，越过两端时绕回。
    fn step(&mut self, down: bool) {
        let level = self.levels.len() - 1;
        let current = &self.levels[level];
        let n = current.rows.len();
        if n == 0 {
            return;
        }
        let start = current.hover;
        let mut i = start.unwrap_or(if down { n - 1 } else { 0 });
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if current.rows[i].selectable() {
                break;
            }
        }
        if current.rows[i].selectable() {
            self.levels[level].hover = Some(i);
            self.reveal(level, i);
        }
    }

    /// 滚动第 `level` 级菜单，使第 `row` 行完整露出来。
    fn reveal(&mut self, level: usize, row: usize) {
        let level = &mut self.levels[level];
        let (Some(rect), true) = (level.row_rects.get(row), level.rect.h > 0.0) else {
            return;
        };
        let top = level.rect.y + POPUP_PAD;
        let bottom = level.rect.bottom() - POPUP_PAD;
        if rect.y < top {
            level.scroll -= top - rect.y;
        } else if rect.bottom() > bottom {
            level.scroll += rect.bottom() - bottom;
        }
        level.scroll = level.scroll.max(0.0);
    }
}

/// 菜单上下的内边距（逻辑像素，经验值）：菜单外框与第一行、最后一行之间各留出的空隙。
pub(crate) const POPUP_PAD: f32 = 5.0;
/// 左边打勾那一列的宽度（逻辑像素）：勾选标记宽 9、中心距列首 13，其余是与菜单外框的空隙。
pub(crate) const CHECK_COLUMN: f32 = 24.0;
/// 有图标的菜单中图标那一列的宽度（逻辑像素）：最宽的图标位 16，加上到文字的间距。
pub(crate) const ICON_COLUMN: f32 = 22.0;
/// 有图标的菜单中文字离菜单左边的距离（逻辑像素）。
pub(crate) const POPUP_TEXT_X: f32 = CHECK_COLUMN + ICON_COLUMN;

/// 返回目录菜单中目录 `dir` 的各行：文件和目录，目录与文件之间一条分隔线。
///
/// 读取失败、空目录或项数过多时有一行灰色的说明（`folder_entries`）。`current` 是当前文件，
/// 它和它的各级上层目录打勾。
pub(crate) fn folder_rows(dir: &Path, current: &Path) -> Vec<PopupRow> {
    use crate::app::status::breadcrumb::{FolderEntry, folder_entries};
    use winc_editor::EntryKind;
    folder_entries(dir, current)
        .into_iter()
        .map(|entry| match entry {
            FolderEntry::Item {
                name,
                path,
                kind,
                checked,
            } => PopupRow::Entry {
                name,
                path,
                dir: kind == EntryKind::Dir,
                checked,
            },
            FolderEntry::Separator => PopupRow::Separator,
            FolderEntry::Note(note) => PopupRow::Item {
                label: MenuLabel::plain(&note),
                icon: None,
                key: String::new(),
                enabled: false,
                checked: false,
                tag: 0,
            },
        })
        .collect()
}

impl App {
    /// 把菜单项 `entries` 变成窗口内菜单的各行；`numbered` 是已经编到的号。
    ///
    /// 编号方式与 [`MenuEntry::command_at`] 相同，因此选中后按编号就能找回命令。
    pub(in crate::app) fn popup_rows(
        &mut self,
        entries: &[MenuEntry],
        context: Context,
        numbered: &mut isize,
    ) -> Vec<PopupRow> {
        let mut rows = Vec::new();
        for entry in entries {
            match entry {
                MenuEntry::Item(command, enabled) => {
                    *numbered += 1;
                    let key = command
                        .shortcut(context)
                        .map(|s| s.label(keymap::Os::CURRENT))
                        .unwrap_or_default();
                    rows.push(PopupRow::Item {
                        label: self.menu_label(*command),
                        icon: self.menu_icon_kind(*command),
                        key,
                        enabled: *enabled,
                        checked: self.menu_checked(*command),
                        tag: *numbered,
                    });
                }
                // 按住 `⌥` 才出现的备选项不显示，只占一个编号。
                MenuEntry::Alternate(..) => *numbered += 1,
                MenuEntry::Submenu(title, items) => {
                    let children = self.popup_rows(items, context, numbered);
                    rows.push(PopupRow::Submenu {
                        label: MenuLabel::plain(title),
                        checked: false,
                        rows: children,
                    });
                }
                MenuEntry::Separator => {
                    // 开头、结尾和连着的分隔线不画。
                    if rows.last().is_some_and(|r| *r != PopupRow::Separator) {
                        rows.push(PopupRow::Separator);
                    }
                }
            }
        }
        if rows.last() == Some(&PopupRow::Separator) {
            rows.pop();
        }
        rows
    }

    /// 窗口内的菜单。
    pub(crate) fn popup(&self) -> Option<&PopupMenu> {
        self.overlays.context_menu.as_ref()?.popup.as_ref()
    }

    fn popup_mut(&mut self) -> Option<&mut PopupMenu> {
        self.overlays.context_menu.as_mut()?.popup.as_mut()
    }

    /// 关掉窗口内的菜单，不执行任何命令。
    pub(in crate::app) fn dismiss_popup(&mut self) {
        if self.popup().is_some() {
            self.overlays.context_menu = None;
            self.overlays.menu_button = None;
            self.set_hover(self.hover_at(self.pointer.mouse.0, self.pointer.mouse.1));
            self.request_redraw();
        }
    }

    /// 选中编号为 `tag` 的命令：关掉菜单，执行它。
    fn choose_popup(&mut self, tag: isize) {
        let command = self
            .overlays
            .context_menu
            .as_ref()
            .and_then(|menu| MenuEntry::command_at(&menu.entries, tag));
        if let Some(menu) = &mut self.overlays.context_menu {
            menu.popup = None;
        }
        if let Some(command) = command {
            self.run_menu_command(command);
        }
        self.overlays.context_menu = None;
        self.overlays.menu_button = None;
        self.request_redraw();
    }

    /// 鼠标移到 `(x, y)`；窗口内的菜单打开时返回 `true`，此时只有菜单响应悬停。
    pub(in crate::app) fn popup_mouse_move(&mut self, x: f32, y: f32) -> bool {
        let Some(popup) = self.popup_mut() else {
            return false;
        };
        if let Some((level, row)) = popup.hit(x, y) {
            let same = popup.levels.len() > level + 1
                && popup.levels[level + 1].parent == row
                && row.is_some();
            // 停在已经展开的子菜单的那一行上：子菜单不动。
            if !same {
                popup.hover(level, row);
            }
        } else if let Some(last) = popup.levels.last_mut() {
            // 移出菜单：最深一级的停留取消，已经展开的子菜单留着。
            last.hover = None;
        }
        self.request_redraw();
        true
    }

    /// 按下鼠标按键 `button`；这次按下由菜单处理时返回 `true`。
    ///
    /// 点在可用的命令上执行它，点在子菜单上展开它；左键点在菜单外面关掉菜单，这次按下不再
    /// 交给下面；右键点在菜单外面时先关掉菜单，再把事件交给下面，在新的位置弹出那里的菜单。
    pub(in crate::app) fn popup_mouse_down(&mut self, button: MouseButton) -> bool {
        let (x, y) = self.pointer.mouse;
        let Some(popup) = self.popup_mut() else {
            return false;
        };
        if button == MouseButton::Right && popup.hit(x, y).is_none() {
            self.dismiss_popup();
            return false;
        }
        let outcome = match popup.hit(x, y) {
            Some((level, Some(row))) => match &popup.levels[level].rows[row] {
                PopupRow::Item {
                    enabled: true, tag, ..
                } => Outcome::Choose(*tag),
                PopupRow::Entry {
                    path, dir: false, ..
                } => Outcome::Open(path.clone()),
                r if r.expands() => {
                    popup.hover(level, Some(row));
                    Outcome::Redraw
                }
                _ => Outcome::Redraw,
            },
            Some((_, None)) => Outcome::Redraw,
            None => Outcome::Dismiss,
        };
        self.popup_outcome(outcome);
        true
    }

    /// 执行菜单上一次操作的结果。
    fn popup_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Redraw => self.request_redraw(),
            Outcome::Choose(tag) => self.choose_popup(tag),
            Outcome::Open(path) => {
                self.dismiss_popup();
                self.open_path(&path);
            }
            Outcome::Dismiss => self.dismiss_popup(),
        }
    }

    /// 滚轮：窗口内的菜单打开时返回 `true`，滚动鼠标下的那一级菜单。
    pub(in crate::app) fn popup_wheel(&mut self, dy: f32) -> bool {
        let (x, y) = self.pointer.mouse;
        let Some(popup) = self.popup_mut() else {
            return false;
        };
        if let Some((level, _)) = popup.hit(x, y) {
            let level = &mut popup.levels[level];
            let max = (level.content - level.rect.h).max(0.0);
            level.scroll = (level.scroll - dy).clamp(0.0, max);
            self.request_redraw();
        }
        true
    }

    /// 按键：窗口内的菜单打开时返回 `true`，菜单之外的快捷键都不处理。
    ///
    /// `↑` `↓` 移动，`→` 展开子菜单，`←` 收起，`↩` 或空格选中，`Esc` 先收起子菜单、
    /// 再关掉菜单。
    pub(in crate::app) fn popup_key(&mut self, event: &KeyEvent) -> bool {
        self.popup_logical_key(&event.logical_key)
    }

    /// 同 [`App::popup_key`]，给的是按键的逻辑键。
    pub(in crate::app) fn popup_logical_key(&mut self, key: &Key) -> bool {
        let Some(popup) = self.popup_mut() else {
            return false;
        };
        let level = popup.levels.len() - 1;
        let hovered = popup.levels[level].hover;
        let row = hovered.map(|r| &popup.levels[level].rows[r]);
        let outcome = match key {
            Key::Named(NamedKey::ArrowDown) => {
                popup.step(true);
                Outcome::Redraw
            }
            Key::Named(NamedKey::ArrowUp) => {
                popup.step(false);
                Outcome::Redraw
            }
            Key::Named(NamedKey::ArrowRight | NamedKey::Enter | NamedKey::Space)
                if row.is_some_and(PopupRow::expands) =>
            {
                popup.hover(level, hovered);
                popup.step(true);
                Outcome::Redraw
            }
            Key::Named(NamedKey::Enter | NamedKey::Space) => match row {
                Some(PopupRow::Item {
                    enabled: true, tag, ..
                }) => Outcome::Choose(*tag),
                Some(PopupRow::Entry {
                    path, dir: false, ..
                }) => Outcome::Open(path.clone()),
                _ => Outcome::Redraw,
            },
            Key::Named(NamedKey::ArrowLeft | NamedKey::Escape) if level > 0 => {
                popup.levels.pop();
                Outcome::Redraw
            }
            Key::Named(NamedKey::Escape) => Outcome::Dismiss,
            _ => Outcome::Redraw,
        };
        self.popup_outcome(outcome);
        true
    }
}

/// 菜单上一次操作的结果。
///
/// 执行命令前要先关掉菜单，因此把结果先记下，关掉菜单之后再做。
enum Outcome {
    /// 只需重画。
    Redraw,
    /// 选中了此编号的命令。
    Choose(isize),
    /// 目录菜单中选中了该文件。
    Open(PathBuf),
    /// 关掉菜单。
    Dismiss,
}
