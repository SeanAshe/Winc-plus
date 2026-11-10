//! 状态栏的路径：每一段都可以点击，点击后列出该目录的内容，子目录可逐级展开。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use winc_editor::{EntryKind, keypath, list_dir};
use winc_i18n::t;
use winc_render::{Color, Image};

use crate::icons;

use super::*;

/// 目录菜单最多列出的项数（项，经验值）。
///
/// 目录内的项过多（例如 `node_modules`）时，菜单难以浏览，构建开销也随之增大；超出的部分
/// 以一条说明文字代替。
const FOLDER_MENU_MAX: usize = 300;

/// 菜单项文字起点到菜单项左边缘的距离（逻辑像素，经验值）。
///
/// 菜单项左侧是打勾标记所在的列。弹出菜单时，位置向左偏移此距离，使打勾项的文字大致对准
/// 被点击的路径段。
pub(super) const MENU_TITLE_INSET: f32 = 21.0;

/// 目录菜单里图标的大小（逻辑像素）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FOLDER_ICON: f32 = 16.0;

/// 目录菜单里的图标：和文件树、改动列表同一套（目录、各类文件的记号），不用系统的
/// 文件图标，菜单和侧栏看起来是一个应用。
///
/// 子菜单在菜单跟踪期间才读，那时拿不到渲染器，所以弹出菜单前把每种记号用白色画一张，读子菜单
/// 时按文件类型的颜色染色。图标只有一种颜色加不同的透明度，染色与直接用那个颜色画效果相同。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct FolderIcons {
    /// 深色配色：文件类型的颜色按它取。
    dark: bool,
    /// 目录图标的颜色。
    folder_color: Color,
    folder: Image,
    files: Vec<(icons::FileGlyph, Image)>,
}

thread_local! {
    /// 弹出目录菜单前画好的图标，菜单关闭后清掉。
    static FOLDER_ICONS: RefCell<Option<FolderIcons>> = const { RefCell::new(None) };
}

/// 把白色画的 `image`（预乘的 ARGB）染成 `color`。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn tint(image: &Image, color: Color) -> Image {
    let channel = |c: f32, a: u32| ((c.clamp(0.0, 1.0) * a as f32).round() as u32).min(a);
    let pixels = image
        .pixels
        .iter()
        .map(|&px| {
            let a = px >> 24;
            (a << 24) | channel(color.r, a) << 16 | channel(color.g, a) << 8 | channel(color.b, a)
        })
        .collect();
    Image {
        pixels,
        width: image.width,
        height: image.height,
    }
}

/// 目录菜单里 `name` 这一项的图标；菜单没有画好图标时为 `None`。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn folder_icon(name: &str, is_dir: bool) -> Option<Image> {
    FOLDER_ICONS.with_borrow(|icons| {
        let icons = icons.as_ref()?;
        if is_dir {
            return Some(tint(&icons.folder, icons.folder_color));
        }
        let (glyph, [dark, light]) = crate::theme::file_kind(name);
        let image = &icons.files.iter().find(|(g, _)| *g == glyph)?.1;
        Some(tint(
            image,
            Color::hex(if icons.dark { dark } else { light }),
        ))
    })
}

/// 状态栏路径中的一段。
pub struct Crumb {
    /// 这一段显示的名称。
    pub name: String,
    /// 这一段对应的目录或文件。
    ///
    /// 未命名的文件为 `None`，不可点击。
    pub path: Option<PathBuf>,
}

/// 目录菜单中的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderEntry {
    /// 一个文件或子目录。
    Item {
        /// 显示名。
        name: String,
        /// 完整路径。
        path: PathBuf,
        /// 是文件还是目录。
        kind: EntryKind,
        /// 是否为当前文件，或当前文件的某一级上层目录。
        checked: bool,
    },
    /// 目录与文件之间的分隔线。
    Separator,
    /// 灰色的说明文字：空目录、读取失败，或项数过多未全部列出。
    Note(String),
}

/// 返回目录 `dir` 在目录菜单中的菜单项。
///
/// 隐藏规则和排序与文件树相同，目录与文件之间加一条分隔线。读取失败、空目录，
/// 以及项数超过 [`FOLDER_MENU_MAX`] 时，以 [`FolderEntry::Note`] 说明。`current` 为当前文件的路径，
/// 用于标记打勾的项。
pub fn folder_entries(dir: &Path, current: &Path) -> Vec<FolderEntry> {
    let children = match list_dir(dir) {
        Ok(children) => children,
        Err(e) => {
            let note = t!("workspace-breadcrumb-read-failed", error = e.to_string());
            return vec![FolderEntry::Note(note)];
        }
    };
    if children.is_empty() {
        return vec![FolderEntry::Note(
            t!("workspace-breadcrumb-empty").to_owned(),
        )];
    }

    let shown = &children[..children.len().min(FOLDER_MENU_MAX)];
    let mut entries = Vec::with_capacity(shown.len() + 3);
    for (i, child) in shown.iter().enumerate() {
        if i > 0 && child.kind == EntryKind::File && shown[i - 1].kind == EntryKind::Dir {
            entries.push(FolderEntry::Separator);
        }
        entries.push(FolderEntry::Item {
            name: child.name.clone(),
            path: child.path.clone(),
            kind: child.kind,
            checked: current.starts_with(&child.path),
        });
    }
    if children.len() > shown.len() {
        entries.push(FolderEntry::Separator);
        let rest = children.len() - shown.len();
        entries.push(FolderEntry::Note(t!(
            "workspace-breadcrumb-more",
            count = rest
        )));
    }
    entries
}

/// 返回点击路径中 `crumb` 一段时应列出的目录。
///
/// 目录段列出该目录自身，当前文件段（`crumb == current`）列出该文件所在的目录；文件没有
/// 上级目录时返回 `None`。
fn crumb_folder<'a>(crumb: &'a Path, current: &Path) -> Option<&'a Path> {
    if crumb == current {
        crumb.parent()
    } else {
        Some(crumb)
    }
}

/// 把 [`folder_entries`] 的结果转换为 macOS 目录菜单的菜单项，`None` 表示分隔线。
///
/// 弹出菜单和展开子菜单时都会调用。
#[cfg(target_os = "macos")]
pub(in crate::app) fn macos_folder_items(
    dir: &Path,
    current: &Path,
) -> Vec<Option<crate::macos::FolderItem>> {
    use crate::macos::FolderItem;

    folder_entries(dir, current)
        .into_iter()
        .map(|entry| match entry {
            FolderEntry::Item {
                name,
                path,
                kind,
                checked,
            } => Some(FolderItem {
                icon: folder_icon(&name, kind == EntryKind::Dir),
                title: name,
                path: Some(path),
                is_dir: kind == EntryKind::Dir,
                checked,
            }),
            FolderEntry::Separator => None,
            FolderEntry::Note(text) => Some(FolderItem {
                icon: None,
                title: text,
                path: None,
                is_dir: false,
                checked: false,
            }),
        })
        .collect()
}

impl App {
    /// 返回当前文件在状态栏上显示的路径段；没有活动文档时返回空列表。
    ///
    /// 文件在已打开的目录内时，从目录名开始逐级列出；不在目录内时为单独一段（主目录显示为 `~`）；
    /// 未命名的文件只有名字一段。
    pub(crate) fn crumbs(&self) -> Vec<Crumb> {
        let Some(doc) = self.editor.active() else {
            return Vec::new();
        };

        // 差异标签没有自己的文件，按它比较的文件显示路径。
        let compared = doc
            .diff
            .as_deref()
            .and_then(|v| v.new.path.clone().or_else(|| v.old.path.clone()));
        match (&doc.path.clone().or(compared), &self.editor.workspace) {
            (Some(path), Some(ws)) if path.starts_with(&ws.root) => {
                let mut at = ws.root.clone();
                let mut crumbs = vec![Crumb {
                    name: ws.name.clone(),
                    path: Some(at.clone()),
                }];
                for part in path.strip_prefix(&ws.root).into_iter().flat_map(Path::iter) {
                    at.push(part);
                    crumbs.push(Crumb {
                        name: part.to_string_lossy().into_owned(),
                        path: Some(at.clone()),
                    });
                }
                crumbs
            }
            (Some(path), _) => vec![Crumb {
                name: winc_editor::display_path(path, None),
                path: Some(path.clone()),
            }],
            (None, _) => vec![Crumb {
                name: doc.name().into_owned(),
                path: None,
            }],
        }
    }

    /// 返回配置文件（JSON、YAML、TOML）中光标所在的键路径，显示在状态栏路径之后。
    ///
    /// 不是这三种语言、文件超过 `KEY_PATH_MAX` 字节，或光标不在键内时返回 `None`。结果按（文档、
    /// 版本号、光标位置）缓存，三者都不变时不重新计算；JSON 沿用上一次的扫描记录继续扫描
    /// （`KeyPathScan`）。
    pub(crate) fn key_path(&mut self) -> Option<String> {
        let doc = self.editor.active()?;
        let language = doc.highlighter.language().id;
        if !matches!(language, "json" | "yaml" | "toml") {
            return None;
        }
        let rope = doc.text().rope()?;
        if rope.len_bytes() > keypath::KEY_PATH_MAX {
            return None;
        }
        let key = (doc.id, doc.revision, doc.selection.head);
        if let Some((cached, path)) = &self.key_path.cache
            && *cached == key
        {
            return path.clone();
        }
        let pos = doc.selection.head;
        let segments = if language == "json" {
            let scan = match &mut self.key_path.scan {
                Some((id, scan)) if *id == doc.id => scan,
                slot => &mut slot.insert((doc.id, keypath::KeyPathScan::default())).1,
            };
            scan.json(rope, pos)
        } else {
            keypath::key_path(rope, language, pos)
        };
        let path = keypath::format(&segments);
        let path = (!path.is_empty()).then_some(path);
        self.key_path.cache = Some((key, path.clone()));
        path
    }

    /// 把光标所在的键路径复制到剪贴板（命令面板中的命令）；光标不在键内时给出提示。
    pub(in crate::app) fn copy_key_path(&mut self) {
        match self.key_path() {
            Some(path) => {
                self.set_clipboard(path.clone(), false);
                self.flash(t!("workspace-key-path-copied", path = path));
            }
            None => self.flash(t!("workspace-key-path-none")),
        }
    }

    /// 用白色画好目录菜单要的图标（见 [`FolderIcons`]）；没有渲染器时为 `None`，菜单没有图标。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn folder_icons(&mut self) -> Option<FolderIcons> {
        let (dark, folder_color) = (self.palette.dark, self.palette.folder);
        let renderer = self.renderer.as_mut()?;
        let s = FOLDER_ICON;
        let clip = Rect::new(0.0, 0.0, s, s);
        let white = Color::hex(0xFFFFFF);
        // 目录图标 15 × 12、文件图标 12 × 15，都放在 16 见方的正中。
        let folder = renderer.draw_transparent(s, s, |frame| {
            icons::folder(frame, (0.5, 2.0), false, white, clip);
        });
        let files = icons::FileGlyph::ALL
            .into_iter()
            .map(|glyph| {
                let image = renderer.draw_transparent(s, s, |frame| {
                    icons::file(frame, (2.0, 0.5), glyph, white, clip);
                });
                (glyph, image)
            })
            .collect();
        Some(FolderIcons {
            dark,
            folder_color,
            folder,
            files,
        })
    }

    /// 处理对路径第 `index` 段（从 0 起）的点击：弹出该段的目录菜单。
    ///
    /// 当前文件及其上层目录项打勾，打勾项对准被点击的段；子目录展开时才读内容。在菜单中选中
    /// 文件时打开它：系统菜单经事件循环回到 [`UserEvent::FolderItem`]，没有系统菜单时（macOS
    /// 以外的平台，以及没有窗口的测试中）在窗口中画出同样的菜单（`PopupRow::Entry`）。
    pub(in crate::app) fn open_crumb_menu(&mut self, index: usize) {
        let Some(current) = self.editor.active().and_then(|d| d.path.clone()) else {
            return;
        };
        let Some(path) = self.crumbs().into_iter().nth(index).and_then(|c| c.path) else {
            return;
        };
        let Some(dir) = crumb_folder(&path, &current) else {
            return;
        };
        let Some(&(rect, _)) = self.hits.crumbs.iter().find(|&&(_, i)| i == index) else {
            return;
        };

        if !cfg!(target_os = "macos") || self.window.handle.is_none() {
            // 打勾那一行的文字对准被点击的段：文字前面是打勾和图标两列。
            let at = (rect.x - POPUP_TEXT_X, rect.y + rect.h / 2.0);
            let mut popup = PopupMenu::new(folder_rows(dir, &current), at, (true, false));
            popup.current = Some(current);
            self.context_menu = Some(ContextMenu {
                target: crate::app::input::context_menu::MenuTarget::Folder,
                entries: Vec::new(),
                branches: Vec::new(),
                stashes: Vec::new(),
                popup: Some(popup),
            });
            self.menu_button = Some(MenuButton::Crumb(index));
            self.set_hover(Hover::None);
            self.request_redraw();
            return;
        }

        // 菜单弹出期间系统在跟踪菜单事件，窗口不会重绘，因此先同步绘制一次，
        // 把这一段显示为按下状态。
        self.menu_button = Some(MenuButton::Crumb(index));
        self.paint_now();

        #[cfg(target_os = "macos")]
        if let Some(window) = self.window.handle.clone() {
            let icons = self.folder_icons();
            FOLDER_ICONS.set(icons);
            let at = (f64::from(rect.x - MENU_TITLE_INSET), f64::from(rect.y));
            crate::macos::popup_folder_menu(&window, dir, &current, at);
            FOLDER_ICONS.set(None);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (dir, rect);

        self.menu_button = None;
        self.request_redraw();
    }
}

#[cfg(test)]
pub(in crate::app) mod tests;

/// 状态栏键路径的缓存与扫描记录。
#[derive(Default)]
pub struct KeyPath {
    /// 状态栏上键路径的缓存，键为（文档、版本号、光标位置）。
    pub(in crate::app) cache: Option<((DocumentId, u64, usize), Option<String>)>,
    /// 当前文档 JSON 键路径的扫描记录；切换文档后重新扫描。
    pub(in crate::app) scan: Option<(DocumentId, winc_editor::keypath::KeyPathScan)>,
}
