//! 菜单项左边的图标：各命令使用的图标种类、图标位的尺寸，以及绘制为菜单使用的图像。

use super::*;

/// 标签列表菜单的图标边长（逻辑像素）。
///
/// 文件图标为 12 × 15，无法放入其他菜单 15 × 12 的图标位，因此改用 16 × 16 的图标位，
/// 状态栏路径的目录菜单也使用该尺寸。
const TAB_LIST_ICON: f32 = 16.0;

/// 菜单项左边的图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuIcon {
    /// 最近打开的目录：与文件树中的目录相同的图标和颜色。
    Folder,
    /// 最近打开的一组目录：两个叠放的目录。
    Folders,
    /// 标签列表的第几项：标签上的图标（文件类型图标，日志标签为 Git 图标）。
    Tab(usize),
    /// 目录菜单中的文件、语言菜单中的语言：文件类型的记号和它在深色、浅色配色下的颜色（同
    /// 文件树）。
    File(icons::FileGlyph, [u32; 2]),
}

impl MenuIcon {
    /// 图标位的逻辑尺寸（宽，高）。
    pub(crate) fn size(self) -> (f32, f32) {
        match self {
            MenuIcon::Tab(_) | MenuIcon::File(..) => (TAB_LIST_ICON, TAB_LIST_ICON),
            MenuIcon::Folder | MenuIcon::Folders => MENU_ICON,
        }
    }
}

impl App {
    /// 返回菜单项左边的图标：最近的目录用目录图标，工作树用分支图标，标签列表用标签上的图标，
    /// 语言用这种语言的文件的图标；其他命令没有图标。
    pub(in crate::app) fn menu_icon_kind(&self, command: MenuCommand) -> Option<MenuIcon> {
        match command {
            MenuCommand::OpenRecent(i) | MenuCommand::OpenRecentInNewWindow(i) => {
                let group = (self.state.recent.get(i))
                    .is_some_and(|folder| self.recent_group(folder).is_some());
                Some(if group {
                    MenuIcon::Folders
                } else {
                    MenuIcon::Folder
                })
            }
            MenuCommand::OpenGroup(_) => Some(MenuIcon::Folders),
            MenuCommand::GroupFolder(..) => Some(MenuIcon::Folder),
            MenuCommand::ShowTab(i) => self.editor.documents.get(i).map(|_| MenuIcon::Tab(i)),
            MenuCommand::UseLanguage(_) | MenuCommand::UsePlainText => {
                let language = match command {
                    MenuCommand::UseLanguage(i) => winc_syntax::languages().get(i).copied()?,
                    _ => winc_syntax::detect(""),
                };
                let (glyph, colors) = crate::theme::language_kind(language);
                Some(MenuIcon::File(glyph, colors))
            }
            _ => None,
        }
    }

    /// 在 `(x, y)`（图标位的左上角）绘制菜单项图标 `icon`，图标位的尺寸见
    /// [`MenuIcon::size`]。
    ///
    /// 窗口的深浅外观跟随主题，菜单跟随窗口，因此图标颜色与菜单的深浅外观一致。
    pub(crate) fn draw_menu_icon(
        &self,
        frame: &mut winc_render::Frame<'_>,
        icon: MenuIcon,
        (x, y): (f32, f32),
        clip: Rect,
    ) {
        let (w, h) = icon.size();
        match icon {
            MenuIcon::Folder => icons::folder(frame, (x, y), false, self.palette.folder, clip),
            MenuIcon::Folders => icons::folders(frame, (x, y), self.palette.folder, clip),
            MenuIcon::File(glyph, [dark, light]) => {
                let color = winc_render::Color::hex(if self.palette.dark { dark } else { light });
                // 文件图标 12 × 15。
                icons::file(frame, (x + 2.0, y + 0.5), glyph, color, clip);
            }
            MenuIcon::Tab(i) => {
                let Some(doc) = self.editor.documents.get(i) else {
                    return;
                };
                if doc.log.is_some() {
                    icons::git(
                        frame,
                        (x + w / 2.0, y + h / 2.0),
                        1.0,
                        self.palette.accent,
                        clip,
                    );
                } else if doc.terminal {
                    icons::terminal(
                        frame,
                        (x + w / 2.0, y + h / 2.0),
                        self.palette.text_muted,
                        clip,
                    );
                } else {
                    let (glyph, color) = self
                        .palette
                        .file_icon(&crate::app::workspace::tab_names::tab_icon_name(doc));
                    // 文件图标 12 × 15。
                    icons::file(frame, (x + 2.0, y + 0.5), glyph, color, clip);
                }
            }
        }
    }

    /// 返回菜单项 `command` 的图标的逻辑尺寸（宽，高）；没有图标时为 [`MENU_ICON`]。
    #[cfg(any(target_os = "macos", test))]
    pub(in crate::app) fn menu_icon_size(&self, command: MenuCommand) -> (f32, f32) {
        (self.menu_icon_kind(command)).map_or(MENU_ICON, MenuIcon::size)
    }

    /// 把菜单项 `command` 的图标画成图片，交给系统菜单；没有图标或没有渲染器时为 `None`。
    #[cfg(any(target_os = "macos", test))]
    pub(in crate::app) fn menu_icon(&mut self, command: MenuCommand) -> Option<Image> {
        let icon = self.menu_icon_kind(command)?;
        let (w, h) = icon.size();
        let clip = Rect::new(0.0, 0.0, w, h);
        let mut renderer = self.frame.renderer.take()?;
        let image = renderer.draw_transparent(w, h, |frame| {
            self.draw_menu_icon(frame, icon, (0.0, 0.0), clip);
        });
        self.frame.renderer = Some(renderer);
        Some(image)
    }
}
