//! 源代码中的链接：`⌘` 单击（其他平台为 `Ctrl` 单击）、右键菜单和命令打开光标处的网址、
//! Markdown 链接或维基链接；按住修饰键时鼠标下的链接加下划线。

use super::*;
use winc_editor::links::{Target as LinkTarget, link_at};
use winc_i18n::t;
use winc_preview::markdown::syntax::WIKI;

/// 查找链接时一行最多读取的字符数；更长的行不查找。
///
/// 取自调参：远长于常见的网址和 Markdown 行，限制鼠标每次移动时读取的文字。
const LINE_MAX: usize = 4096;

/// 源代码中的一处链接。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextLink {
    /// 所在的文档及其版本号；文档修改后位置不再有效。
    pub doc: DocumentId,
    pub revision: u64,
    /// 链接的起止位置（字符偏移）。
    pub from: usize,
    pub to: usize,
    /// 地址，写法同预览中链接的地址：维基链接以 [`WIKI`] 开头。
    pub url: String,
}

impl App {
    /// 打开链接的修饰键是否按下：macOS 上是 `⌘`，其他平台是 `Ctrl`。
    pub(in crate::app) fn link_modifier(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.pointer.modifiers.super_key()
        } else {
            self.pointer.modifiers.control_key()
        }
    }

    /// 返回当前文档中位置 `pos` 处的链接；Markdown 链接和维基链接只在 Markdown 文件中查找。
    pub(in crate::app) fn text_link_at(&self, pos: usize) -> Option<TextLink> {
        let doc = self.editor.active().filter(|doc| doc.shows_text())?;
        let text = doc.text();
        let (line, col) = text.pos_to_line_col(pos);
        let len = text.line_len(line);
        if len > LINE_MAX {
            return None;
        }
        let start = pos - col;
        let content = doc.text_range(start, start + len)?;
        let markdown = doc.highlighter.language().id == "markdown";
        let link = link_at(&content, col, markdown)?;
        let url = match link.target {
            LinkTarget::Address(address) => address,
            LinkTarget::Wiki(name) => format!("{WIKI}{name}"),
        };
        Some(TextLink {
            doc: doc.id,
            revision: doc.revision,
            from: start + link.cols.start,
            to: start + link.cols.end,
            url,
        })
    }

    /// 返回光标处的链接。
    pub(in crate::app) fn link_at_caret(&self) -> Option<TextLink> {
        let caret = self.editor.active()?.selection.head;
        self.text_link_at(caret)
    }

    /// 打开光标处的链接，返回光标是否在链接上。
    pub(in crate::app) fn open_link_at_caret(&mut self) -> bool {
        let Some(link) = self.link_at_caret() else {
            return false;
        };
        self.open_link(&link.url);
        true
    }

    /// 打开光标处的链接；光标不在链接上时提示。
    pub(in crate::app) fn open_caret_link(&mut self) {
        if !self.open_link_at_caret() {
            self.flash(t!("editor-no-link"));
        }
    }

    /// 记下按住修饰键时鼠标下的链接，供绘制下划线；鼠标移动、修饰键变化时调用。
    pub(in crate::app) fn update_text_link(&mut self) {
        let (x, y) = self.pointer.mouse;
        let link = (self.link_modifier() && self.over_editor_text())
            .then(|| self.pos_at(x, y))
            .flatten()
            .and_then(|pos| self.text_link_at(pos));
        if self.pointer.link != link {
            self.pointer.link = link;
            self.request_redraw();
        }
    }

    /// 在源代码中跳到锚点 `anchor` 对应的标题行，返回是否找到；锚点的算法同预览。
    pub(in crate::app) fn go_to_heading(&mut self, anchor: &str) -> bool {
        let Some(text) = self
            .editor
            .active()
            .and_then(|doc| doc.text_range(0, usize::MAX))
        else {
            return false;
        };
        let wanted = crate::app::preview::markdown::percent_decode(anchor).to_lowercase();
        let keys = winc_preview::markdown::parse(&text).heading_keys();
        let Some(nth) = (keys.iter().flatten()).position(|key| *key == wanted) else {
            return false;
        };
        // 第 `nth` 个标题所在的行：1 到 6 个 `#` 加空白开头的行，或下一行全是 `=`、`-` 的行；
        // 代码块中的除外。
        let lines: Vec<&str> = text.lines().collect();
        let ruled = |line: &str| {
            let line = line.trim();
            !line.is_empty() && (line.bytes().all(|b| b == b'=') || line.bytes().all(|b| b == b'-'))
        };
        let mut fenced = false;
        let mut heading = |at: usize| {
            let line = lines[at].trim_start();
            if line.starts_with("```") || line.starts_with("~~~") {
                fenced = !fenced;
                return false;
            }
            let title = line.trim_start_matches('#');
            let level = line.len() - title.len();
            let atx = (1..=6).contains(&level) && !title.starts_with(|c: char| !c.is_whitespace());
            let underlined = level == 0
                && !line.is_empty()
                && !ruled(line)
                && lines.get(at + 1).is_some_and(|next| ruled(next));
            !fenced && (atx || underlined)
        };
        let Some(line) = (0..lines.len()).filter(|&at| heading(at)).nth(nth) else {
            return false;
        };
        self.go_to(line, 0, 0);
        true
    }
}
