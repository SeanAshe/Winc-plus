//! Markdown 中补全链接和图片的地址：在 `](` 之后输入时，在光标下方列出打开的目录里的文件，
//! 按已输入的文字筛选（打分与快速打开相同），图片只列出图片，选中后写入相对文档的路径；地址
//! 以 `#` 开头时列出本文的标题，写入锚点。
//!
//! 在行首或空白之后输入 `/` 时列出写作命令（见 `slash`）。
//!
//! 输入的文字仍写入文档，列表只处理 ↑↓、回车、Tab 和 Esc。文件来自快速打开的文件表，不另外
//! 遍历磁盘。

use winc_preview::markdown::paragraph::Target;
use winit::keyboard::{Key, NamedKey};

use super::*;

/// 列出的文件最多几个。
pub(crate) const SUGGEST_ROWS: usize = 8;
/// 列表最多显示的行数；项数更多时可以滚动。
pub(crate) const SUGGEST_VISIBLE: usize = 10;
/// 往光标前看多少个字找 `](`：地址写在一行里，很长的一行不必整行读出。
const LOOK_BACK: usize = 512;

/// 列出的一项。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Hit {
    /// 文件：在文件表中的路径（相对打开的目录，见 `IndexedFile`）和绝对路径。
    File(String, PathBuf),
    /// 本文的标题：级别、文字和锚点。
    Heading(u8, String, String),
    /// 写作命令。
    Command(super::slash::Slash),
}

/// 补全的对象：链接地址，或 `/` 之后的命令名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Link(Target),
    Command,
}

/// 补全地址的状态。
#[derive(Default)]
pub(crate) struct Suggest {
    /// 正在补全的内容：文档、开头的位置（字符偏移）和补全的对象。
    pub(crate) at: Option<(DocumentId, usize, Kind)>,
    /// 列出的各项，最多 [`SUGGEST_ROWS`] 个。
    pub(crate) hits: Vec<Hit>,
    /// 选中的一项。
    pub(crate) selected: usize,
    /// 算出 `hits` 时的依据：已输入的地址和文件表的文件数；变了才重新筛选。
    basis: Option<(String, usize)>,
    /// 按 Esc 关掉的地址：光标离开这个地址之前不再列出。
    dismissed: Option<(DocumentId, usize)>,
    /// 上一次看到的文档版本：版本变了（在输入）才打开列表，只移动光标不打开。
    seen: Option<(DocumentId, u64)>,
    /// 最近使用的命令，最近的在前。
    pub(crate) recent: Vec<super::slash::Slash>,
    /// 列表显示的第一项：项数多于可见行数时可以滚动，用键盘换选中项时随之滚动。
    pub(crate) first: usize,
    /// 滚轮累计的、不足一行的位移（逻辑像素）。
    wheel: f32,
}

impl App {
    /// 每帧绘制之前调用：光标在链接地址里输入时列出文件，离开地址时收起。
    pub(in crate::app) fn track_suggest(&mut self) {
        let now = self.editor.active().map(|d| (d.id, d.revision));
        let suggest = &mut self.preview.markdown.suggest;
        // 列表关着、文档也没改：不必读光标前的文字。
        if suggest.at.is_none() && suggest.dismissed.is_none() && suggest.seen == now {
            return;
        }
        let Some((id, revision, start, kind, query)) = self.suggest_context() else {
            let suggest = &mut self.preview.markdown.suggest;
            (suggest.at, suggest.dismissed, suggest.seen) = (None, None, now);
            return;
        };
        let suggest = &mut self.preview.markdown.suggest;
        // 同一个文档的版本变了才算在输入；换到另一个文档不算。
        let typed = suggest
            .seen
            .is_some_and(|(seen, old)| seen == id && old != revision);
        suggest.seen = Some((id, revision));
        if suggest.dismissed == Some((id, start)) {
            return;
        }
        let same = suggest.at == Some((id, start, kind));
        if !same && !typed {
            suggest.at = None;
            return;
        }
        if !same {
            (suggest.at, suggest.selected, suggest.basis) = (Some((id, start, kind)), 0, None);
            if matches!(kind, Kind::Link(Target::File | Target::Image)) {
                self.prepare_index();
            }
        }
        let basis = (query, self.overlays.picker.index.len());
        let old = self.preview.markdown.suggest.basis.as_ref();
        if old == Some(&basis) {
            return;
        }
        // 输入变了时回到第一项：重新排过的列表里原来那一行是另一个文件。
        let refiltered = old.is_some_and(|(query, _)| *query != basis.0);
        let hits = match kind {
            Kind::Command => self.command_hits(&basis.0),
            Kind::Link(Target::Anchor) => self.heading_hits(&basis.0),
            Kind::Link(link) => self.file_hits(&basis.0, link == Target::Image),
        };
        let suggest = &mut self.preview.markdown.suggest;
        let selected = if refiltered { 0 } else { suggest.selected };
        suggest.selected = selected.min(hits.len().saturating_sub(1));
        if refiltered {
            suggest.first = 0;
        }
        (suggest.hits, suggest.basis) = (hits, Some(basis));
    }

    /// 返回打开的目录中匹配 `query` 的文件，不含当前文档；`image` 为真时只返回图片。
    fn file_hits(&mut self, query: &str, image: bool) -> Vec<Hit> {
        let own = self.editor.active().and_then(|d| d.path.clone());
        let index = &mut self.overlays.picker.index;
        (index.search(query).into_iter())
            .filter_map(|i| Some((index.file(i)?.path.to_owned(), index.absolute(i)?)))
            .filter(|(_, path)| {
                Some(path) != own.as_ref() && (!image || winc_preview::image::is_image(path))
            })
            .take(SUGGEST_ROWS)
            .map(|(path, abs)| Hit::File(path, abs))
            .collect()
    }

    /// 返回本文中文字或锚点含有 `query`（不区分大小写）的标题。
    ///
    /// 预览已解析本文时取解析结果中的标题，否则只解析各行中的 ATX 标题。
    fn heading_hits(&self, query: &str) -> Vec<Hit> {
        let Some(doc) = self.editor.active() else {
            return Vec::new();
        };
        let md = &self.preview.markdown;
        let headings = match md.parsed.as_ref().filter(|_| md.doc == Some(doc.id)) {
            Some(parsed) => parsed.headings(),
            None => {
                let Some(rope) = doc.text().rope() else {
                    return Vec::new();
                };
                // 只复制以 `#`、反引号、`~` 开头的行：结果只取决于标题和围栏行。
                let lines: Vec<String> = (rope.lines())
                    .filter(|l| matches!(l.chars().find(|&c| c != ' '), Some('#' | '`' | '~')))
                    .map(|l| l.to_string())
                    .collect();
                winc_preview::markdown::paragraph::headings(lines.iter().map(String::as_str))
            }
        };
        let query = query.trim_start_matches('#').to_lowercase();
        (headings.into_iter())
            .filter(|(_, text, slug)| {
                text.to_lowercase().contains(&query) || slug.to_lowercase().contains(&query)
            })
            .take(SUGGEST_ROWS)
            .map(|(level, text, slug)| Hit::Heading(level, text, slug))
            .collect()
    }

    /// 返回光标所在链接地址的上下文：文档、版本号、地址开头（字符偏移）、补全对象和已输入的地址。
    fn suggest_context(&self) -> Option<(DocumentId, u64, usize, Kind, String)> {
        if self.focus != Focus::Editor || !self.markdown_writing() {
            return None;
        }
        let doc = self.editor.active()?;
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return None;
        }
        let rope = doc.text().rope()?;
        let caret = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(caret);
        let from = rope.line_to_char(line).max(caret.saturating_sub(LOOK_BACK));
        let before = rope.slice(from..caret).to_string();
        let link = winc_preview::markdown::paragraph::link_target(&before);
        let (start, kind) = match link {
            Some((start, target)) => (start, Kind::Link(target)),
            None => {
                let at = winc_preview::markdown::paragraph::slash(&before)?;
                self.prose_spot()?;
                (at + 1, Kind::Command)
            }
        };
        // 补全文件需要打开的目录；没有时只补全本文的标题。
        let files = matches!(kind, Kind::Link(Target::File | Target::Image));
        if files && self.editor.root().is_none() {
            return None;
        }
        let query = before[start..].to_owned();
        let start = from + before[..start].chars().count();
        Some((doc.id, doc.revision, start, kind, query))
    }

    /// 列表画出来了时处理不带修饰键的 ↑↓（换选中的一项）、回车和 Tab（写入）、Esc（关掉），
    /// 返回是否处理了这个键。
    ///
    /// 输入法正在组字、Vim 不在插入模式时不接管：按键交给输入法和 Vim。
    pub(in crate::app) fn suggest_key(&mut self, key: &Key) -> bool {
        let m = self.pointer.modifiers;
        let modified = m.shift_key() || m.control_key() || m.alt_key() || m.super_key();
        let shown = self.hits.suggest_card.is_some();
        let composing = !self.preedit_at(Focus::Editor).is_empty();
        if self.focus != Focus::Editor || !shown || modified || composing || !self.vim_typing() {
            return false;
        }
        let suggest = &mut self.preview.markdown.suggest;
        let Some((id, start, _)) = suggest.at else {
            return false;
        };
        let n = suggest.hits.len();
        if n == 0 {
            return false;
        }
        match key {
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let down = matches!(key, Key::Named(NamedKey::ArrowDown));
                let step = if down { 1 } else { n - 1 };
                suggest.selected = (suggest.selected + step) % n;
                // 选中项移出可见范围时滚动列表。
                let (selected, shown) = (suggest.selected, SUGGEST_VISIBLE.min(n));
                suggest.first = suggest
                    .first
                    .min(selected)
                    .max((selected + 1).saturating_sub(shown));
            }
            Key::Named(NamedKey::Enter | NamedKey::Tab) => {
                let selected = suggest.selected;
                self.accept_suggest(selected);
            }
            Key::Named(NamedKey::Escape) => {
                (suggest.at, suggest.dismissed) = (None, Some((id, start)));
            }
            _ => return false,
        }
        // 关掉以后到下一次绘制之前，画过的列表不再接管按键。
        if self.preview.markdown.suggest.at.is_none() {
            self.hits.suggest_card = None;
        }
        self.request_redraw();
        true
    }

    /// 鼠标 `(x, y)` 在列表上、项数多于可见行数时按滚轮位移 `dy` 滚动列表，返回是否已处理。
    pub(in crate::app) fn scroll_suggest(&mut self, (x, y): (f32, f32), dy: f32) -> bool {
        let over = self
            .hits
            .suggest_card
            .is_some_and(|card| card.contains(x, y));
        let suggest = &mut self.preview.markdown.suggest;
        let n = suggest.hits.len();
        if !over {
            return false;
        }
        if n <= SUGGEST_VISIBLE {
            return true;
        }
        let Some(row) = self
            .hits
            .suggest
            .first()
            .map(|(r, _)| r.h)
            .filter(|h| *h > 0.0)
        else {
            return true;
        };
        suggest.wheel -= dy;
        let steps = (suggest.wheel / row).trunc();
        suggest.wheel -= steps * row;
        let first = suggest.first as isize + steps as isize;
        suggest.first = first.clamp(0, (n - SUGGEST_VISIBLE) as isize) as usize;
        self.request_redraw();
        true
    }

    /// 用第 `i` 项替换已输入的地址（文件写相对路径，标题写 `#锚点`），补上缺少的 `)`，
    /// 并把光标移到 `)` 之后。
    pub(in crate::app) fn accept_suggest(&mut self, i: usize) {
        let suggest = &mut self.preview.markdown.suggest;
        let (Some((id, start, _)), Some(hit)) = (suggest.at, suggest.hits.get(i)) else {
            return;
        };
        let hit = hit.clone();
        suggest.at = None;
        self.hits.suggest_card = None;
        if self.editor.active().is_none_or(|d| d.id != id) {
            return;
        }
        // 命令从 `/` 起替换。
        if let Hit::Command(command) = hit {
            self.run_slash(command, start.saturating_sub(1));
            if self.markdown_live() {
                self.preview.markdown.goal = None;
                self.start_live_editing();
            }
            self.request_redraw();
            return;
        }
        let root = self.editor.root().map(Path::to_path_buf);
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let dir = doc.path.as_deref().and_then(Path::parent);
        let target = match &hit {
            Hit::File(_, file) => winc_editor::paste::link_target(dir, root.as_deref(), file),
            Hit::Heading(_, _, slug) => format!("#{slug}"),
            Hit::Command(_) => return,
        };
        let caret = doc.selection.head.min(rope.len_chars());
        let closed = caret < rope.len_chars() && rope.char(caret) == ')';
        let text = if closed { target } else { format!("{target})") };
        let end = start + text.chars().count() + usize::from(closed);
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(start, caret);
        }
        self.run(winc_editor::Command::Paste(text));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::point(end);
        }
        let suggest = &mut self.preview.markdown.suggest;
        suggest.dismissed = Some((id, start));
        if self.markdown_live() {
            self.preview.markdown.goal = None;
            self.start_live_editing();
        }
        self.request_redraw();
    }
}
