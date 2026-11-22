//! 预览中的链接：查找鼠标下的链接，点击时打开网址、跳到标题、打开本地文件，或者执行预览自己
//! 的控件（复制代码、开合折叠块、收起标题、勾选任务）。

use super::*;
use winc_preview::markdown::syntax::WIKI;

/// 正文链接无法打开的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Broken {
    /// 指向的本地文件不存在。
    File,
    /// 文中没有 `#锚点` 对应的标题。
    Heading,
}

impl Broken {
    /// 返回悬停提示中显示的原因。
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Broken::File => t!("preview-markdown-link-file-not-found"),
            Broken::Heading => t!("preview-markdown-heading-not-found"),
        }
    }
}

/// 已检查的链接：所属文档、版本号、解析的版本号和各地址的结果。
#[derive(Debug, Default)]
pub(crate) struct LinkChecks {
    of: Option<(DocumentId, u64, Option<u64>)>,
    results: HashMap<String, Option<Broken>>,
}

/// 两个文件名是否相同，不区分大小写。
fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
        || (!a.is_ascii()
            && a.chars()
                .flat_map(char::to_lowercase)
                .eq(b.chars().flat_map(char::to_lowercase)))
}

impl LinkChecks {
    /// 返回已检查的地址 `url` 的结果。
    pub(crate) fn get(&self, url: &str) -> Option<Broken> {
        self.results.get(url).copied().flatten()
    }

    /// 清空结果，磁盘上的文件变化后调用。
    pub(crate) fn clear(&mut self) {
        self.results.clear();
    }
}

impl App {
    /// 返回正文链接 `url` 无法打开的原因，结果按文档版本号缓存；网址、脚注和远端文件不检查。
    pub(crate) fn check_markdown_link(&mut self, url: &str) -> Option<Broken> {
        let doc = self.editor.active()?;
        let of = Some((doc.id, doc.revision, self.preview.markdown.checked));
        if self.preview.markdown.links.of != of {
            self.preview.markdown.links = LinkChecks {
                of,
                results: HashMap::new(),
            };
        }
        if let Some(&known) = self.preview.markdown.links.results.get(url) {
            return known;
        }
        let broken = match url.strip_prefix('#') {
            Some(anchor) if anchor.starts_with("fn-") || anchor.starts_with("fnref-") => None,
            // 收起的一节中的块没有排版，同时按解析结果中的标题查找。
            Some(anchor) => {
                let key = percent_decode(anchor).to_lowercase();
                let parsed = self.preview.markdown.parsed.as_ref();
                let titled = parsed.is_some_and(|p| p.heading_keys().contains(&Some(key)));
                (laid::anchor_box(self, anchor).is_none() && !titled).then_some(Broken::Heading)
            }
            None if url.starts_with(WIKI) => {
                (self.wiki_target(&url[WIKI.len()..]).is_none()).then_some(Broken::File)
            }
            None => (doc.path.as_deref())
                .and_then(|path| local_target(path, url))
                .filter(|target| !self.may_exist(target))
                .map(|_| Broken::File),
        };
        (self.preview.markdown.links.results).insert(url.to_owned(), broken);
        broken
    }

    /// 返回预览上 `(x, y)` 处的链接序号（`hits.markdown_links` 中的下标）。
    pub(crate) fn markdown_link_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.preview.markdown.area.contains(x, y) {
            return None;
        }
        self.hits
            .markdown_links
            .iter()
            .position(|(rect, _)| rect.contains(x, y))
    }

    /// 第 `index` 个链接是否为预览自身的控件（代码块的复制按钮、图的全屏按钮、折叠块的标题、
    /// 勾选框），而非正文中的链接。
    pub(crate) fn markdown_link_is_control(&self, index: usize) -> bool {
        use crate::app::preview::markdown::laid::{COPY, DETAILS, EXPAND, FOLD, TABLE, TASK};
        self.hits.markdown_links.get(index).is_some_and(|(_, url)| {
            [COPY, DETAILS, EXPAND, FOLD, TABLE, TASK]
                .iter()
                .any(|prefix| url.starts_with(prefix))
        })
    }

    /// 处理预览中链接的点击：网址交给浏览器，`#标题` 滚动到该标题，本地文件在编辑器中打开。
    pub(crate) fn open_markdown_link(&mut self, index: usize) {
        let Some((_, url)) = self.hits.markdown_links.get(index).cloned() else {
            return;
        };
        if url == crate::app::preview::markdown::laid::EXPAND {
            self.open_diagram_view();
            return;
        }
        if url == crate::app::preview::markdown::laid::COPY {
            let Some(code) = self.hits.markdown_copy.clone() else {
                return;
            };
            self.set_clipboard(code.to_string(), false);
            self.flash(t!("preview-markdown-code-copied"));
            return;
        }
        if let Some(op) = url.strip_prefix(crate::app::preview::markdown::laid::TABLE) {
            self.live_table_tool(op);
            return;
        }
        if let Some(at) = url.strip_prefix(crate::app::preview::markdown::laid::TASK) {
            if let Ok(at) = at.parse() {
                self.toggle_markdown_task(at);
            }
            return;
        }
        if let Some(key) = url.strip_prefix(crate::app::preview::markdown::laid::FOLD) {
            self.toggle_heading_fold(key);
            return;
        }
        if let Some(id) = url.strip_prefix(crate::app::preview::markdown::laid::DETAILS) {
            if let Ok(id) = u64::from_str_radix(id, 16)
                && !self.preview.markdown.toggled.remove(&id)
            {
                self.preview.markdown.toggled.insert(id);
            }
            self.preview.markdown.toggled_now = Some(url);
            return;
        }
        self.open_link(&url);
    }

    /// 打开地址 `url`：维基链接按名字查找文件，`#标题` 跳到该标题，本地文件在编辑器中打开，
    /// 其余交给系统。
    pub(crate) fn open_link(&mut self, url: &str) {
        if let Some(name) = url.strip_prefix(WIKI) {
            match self.wiki_target(name) {
                Some(path) => self.open_markdown_file(&path, name),
                None => self.flash(t!("preview-markdown-link-file-not-found")),
            }
            return;
        }
        if let Some(anchor) = url.strip_prefix('#') {
            // 没有显示预览时在源代码中跳到标题行。
            if shown_area(self).is_some() {
                self.jump_to_anchor(anchor);
            } else if !self.go_to_heading(anchor) {
                self.flash(t!("preview-markdown-heading-not-found"));
            }
            return;
        }
        let local = self
            .editor
            .active()
            .and_then(|d| d.path.as_deref())
            .and_then(|path| local_target(path, url));
        match local {
            Some(path) if self.may_be_file(&path) => self.open_markdown_file(&path, url),
            Some(_) => self.flash(t!("preview-markdown-link-file-not-found")),
            None => {
                if let Err(e) = system::open_url(url) {
                    self.flash(t!(
                        "preview-markdown-link-open-failed",
                        error = e.to_string()
                    ));
                }
            }
        }
    }

    /// 返回维基链接 `name`（可以带 `#标题`）指向的文件；找不到时返回 `None`。
    ///
    /// 没有扩展名时按 `.md` 查找。在文件表中按文件名（不区分大小写）查找，名字中有 `/` 时
    /// 按路径的末尾查找；有多个时取与当前文档共同的上级目录最多的，其次取路径最短的。文件表
    /// 中没有时取当前文档所在目录中的同名文件。
    pub(crate) fn wiki_target(&self, name: &str) -> Option<PathBuf> {
        let name = name.split('#').next().unwrap_or_default().trim();
        if name.is_empty() {
            return self.editor.active().and_then(|doc| doc.path.clone());
        }
        // 名字中的 `.` 不一定是扩展名：按原名找不到时再按补上 `.md` 的名字找。
        let named = format!("{name}.md");
        match Path::new(name).extension() {
            Some(_) => self.wiki_file(name).or_else(|| self.wiki_file(&named)),
            None => self.wiki_file(&named),
        }
    }

    /// 返回文件名（或路径的末尾）为 `wanted` 的文件，规则见 [`Self::wiki_target`]。
    fn wiki_file(&self, wanted: &str) -> Option<PathBuf> {
        let wanted = wanted.replace('\\', "/");
        let here = (self.editor.active()).and_then(|doc| doc.path.as_deref()?.parent());
        let index = &self.overlays.picker.index;
        // 每个文件比较一次，不分配内存：先比长度和路径的边界，再不区分大小写地比末尾。
        let matches = |path: &str| {
            let Some(at) = path.len().checked_sub(wanted.len()) else {
                return false;
            };
            (at == 0 || path.as_bytes()[at - 1] == b'/')
                && path.is_char_boundary(at)
                && same_name(&path[at..], &wanted)
        };
        let found = (0..index.len())
            .filter(|&i| index.file(i).is_some_and(|file| matches(file.path)))
            .filter_map(|i| index.absolute(i));
        let shared = |path: &PathBuf| {
            let near = here.map_or(0, |here| {
                (path.components().zip(here.components()))
                    .take_while(|(a, b)| a == b)
                    .count()
            });
            (std::cmp::Reverse(near), path.components().count())
        };
        found.min_by_key(shared).or_else(|| {
            let sibling = here?.join(&wanted);
            self.may_be_file(&sibling).then_some(sibling)
        })
    }

    /// 切换原文 `at` 处（字节偏移）的任务列表勾选框：`[ ]` 与 `[x]` 互换。
    ///
    /// 修改可以撤销；光标和选区不变，可编辑的预览也不因此进入编辑状态。
    /// 该处不是勾选框（解析结果落后于文字）时不修改。
    fn toggle_markdown_task(&mut self, at: usize) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let mark = rope
            .get_byte_slice(at..at.saturating_add(3).min(rope.len_bytes()))
            .map(|s| s.to_string());
        let Some(mark) = mark else {
            return;
        };
        let next = match mark.as_str() {
            "[ ]" => "x",
            "[x]" | "[X]" => " ",
            _ => return,
        };
        let (id, pos) = (doc.id, rope.byte_to_char(at) + 1);
        let (selections, primary) = doc.selections();
        let editing = self.preview.markdown.editing == Some(id);
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(pos, pos + 1);
        }
        self.run(winc_editor::Command::Insert(next.to_owned()));
        if let Some(doc) = self.editor.active_mut() {
            // 替换的只有一个字符，其前后的位置均不变。
            doc.set_selections(selections, primary);
        }
        if !editing {
            self.preview.markdown.editing = None;
            self.preview.markdown.followed = self
                .live_caret_source()
                .zip(self.editor.active())
                .map(|(source, d)| (d.id, source, d.revision));
        }
        self.request_redraw();
    }
}
