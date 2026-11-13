//! 命令行的输入条与交给应用执行的命令：历史、补全、寄存器的插入，文件与标签的命令、选项、
//! 列表，以及配置中的映射。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use winc_config::{RenderWhitespace, VimMapMode};
use winc_core::{Lines, Selection};
use winc_editor::DocumentId;
use winc_i18n::t;
use winc_vim::ex::{Completing, complete};
use winc_vim::{AppCommand, HostOption, Listing, Mapped, Mapping, Mode, Prompt, Vim, notation};
use winit::keyboard::{Key, NamedKey};

use super::shared;
use crate::app::{App, CloseRequest, Focus, PromptKind, SavePrompt};
use crate::field::TextField;
use crate::keymap::Action;

/// 补全路径时列出的目录项上限（项）。
///
/// 取自调参：候选逐个轮换，更多的候选没有意义。
const MAX_PATHS: usize = 200;

/// 列表中一行预览文字的长度上限（字符）。
const PREVIEW_CHARS: usize = 120;

/// 映射等待后续按键的默认时间（毫秒）。
const TIMEOUT_LEN: u32 = 1000;

/// 复制之后高亮的默认时间（毫秒）。
const YANK_HIGHLIGHT: u32 = 150;

/// 输入条中进行中的历史翻阅与补全。
#[derive(Debug, Default)]
pub(super) struct Line {
    /// 翻阅历史时输入的前缀，以及当前条目在历史中的下标。
    recall: Option<(String, usize)>,
    /// 进行中的补全。
    completion: Option<Completion>,
    /// 是否已按下 `Ctrl-R`，等待寄存器名。
    register: bool,
}

/// 输入条中进行中的补全。
#[derive(Debug)]
struct Completion {
    /// 被补全部分在输入条文字中的起点（字节偏移）。
    start: usize,
    /// 候选。
    candidates: Vec<String>,
    /// 当前候选的下标。
    index: usize,
    /// 填入当前候选之后输入条的文字。
    shown: String,
}

/// `:set` 改动过的选项在配置文件中的值：保存配置时写入这些值，改动只在本次运行中生效。
#[derive(Debug, Default)]
pub(super) struct Kept {
    /// 是否显示行号。
    line_numbers: Option<Option<bool>>,
    /// 是否自动换行。
    word_wrap: Option<Option<bool>>,
    /// 何时显示空白字符。
    whitespace: Option<Option<RenderWhitespace>>,
    /// 相对行号在本次运行中的值；`None` 表示按配置。
    relative: Option<bool>,
}

/// 列表中的一行选中之后的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VimTarget {
    /// 粘贴寄存器。
    Register(char),
    /// 跳转到标记。
    Mark(char),
    /// 切换到文档。
    Tab(DocumentId),
}

/// 列表中的一行。
#[derive(Debug, Clone)]
pub(crate) struct VimRow {
    /// 名字：寄存器名、标记名或文档的序号。
    pub label: String,
    /// 内容的预览。
    pub text: String,
    /// 选中之后的动作。
    pub target: VimTarget,
}

/// `:reg`、`:marks`、`:ls` 在列表浮层中显示的内容。
#[derive(Debug, Clone)]
pub(crate) struct VimList {
    /// 种类。
    pub kind: Listing,
    /// 各行。
    pub rows: Vec<VimRow>,
}

/// 返回文字 `text` 的第一行，过长时截短。
fn preview(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    let mut shown: String = line.chars().take(PREVIEW_CHARS).collect();
    if text.contains('\n') || line.chars().count() > PREVIEW_CHARS {
        shown.push('…');
    }
    shown
}

impl App {
    /// 输入条是否为 Vim 模式的命令行或查找。
    pub(crate) fn vim_prompt_open(&self) -> bool {
        self.bars.goto_input.is_some() && self.bars.prompt_kind.is_vim()
    }

    /// 打开命令行或查找模式的输入条。
    pub(super) fn vim_open_prompt(&mut self, prompt: Prompt) {
        let mut field = TextField::default();
        let kind = match prompt {
            Prompt::Search { forward } => PromptKind::VimSearch { forward },
            Prompt::Command => PromptKind::VimCommand,
        };
        if let Some(state) = &mut self.vim {
            let prefill = state.vim.prompt_prefill().to_owned();
            field.set_text(&prefill);
            field.set_caret(prefill.len(), false);
            state.previewed = Some(String::new());
            state.highlight |= kind != PromptKind::VimCommand;
            state.line = Line::default();
        }
        self.open_prompt(kind, field);
    }

    /// 把输入条的文字换成 `text`，光标放在末尾。
    fn vim_line_set(&mut self, text: &str) {
        if let Some(field) = &mut self.bars.goto_input {
            field.set_text(text);
            field.set_caret(text.len(), false);
        }
        self.request_redraw();
    }

    /// 处理命令行输入条特有的按键：上下键按前缀翻阅历史，`Tab` 补全，`Ctrl-R` 之后的按键插入
    /// 寄存器或光标处的词。返回按键是否已处理。
    pub(in crate::app) fn vim_prompt_key(&mut self, key: &Key, base: &str) -> bool {
        let Some(mut line) = self
            .vim
            .as_mut()
            .map(|state| std::mem::take(&mut state.line))
        else {
            return false;
        };
        let ctrl = self.pointer.modifiers.control_key();
        let handled = if std::mem::take(&mut line.register) {
            let name = base.chars().next().filter(|_| base.chars().count() == 1);
            if let Some(text) = name.and_then(|name| self.vim_insertion(name, ctrl)) {
                self.edit_field(|field| field.insert(&text));
            }
            true
        } else if ctrl && base == "r" {
            line.register = true;
            true
        } else {
            match key {
                Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) => {
                    let older = *key == Key::Named(NamedKey::ArrowUp);
                    self.vim_recall(&mut line, older);
                    true
                }
                Key::Named(NamedKey::Tab) if self.bars.prompt_kind == PromptKind::VimCommand => {
                    self.vim_complete(&mut line, self.pointer.modifiers.shift_key());
                    true
                }
                // 单独的修饰键不打断翻阅与补全。
                Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt) => true,
                _ => {
                    line.recall = None;
                    line.completion = None;
                    false
                }
            }
        };
        if let Some(state) = &mut self.vim {
            state.line = line;
        }
        handled
    }

    /// 返回 `Ctrl-R` 之后的按键要插入的文字：`Ctrl-W` 是光标处的词，其余是该寄存器的第一行。
    fn vim_insertion(&mut self, name: char, ctrl: bool) -> Option<String> {
        let mut state = self.vim.take()?;
        let result = self.vim_with(&mut state, |vim, shared, host| {
            if ctrl && name == 'w' {
                vim.word_at_cursor(host)
            } else {
                Vim::register_text(shared, host, name)
            }
        });
        self.vim = Some(state);
        let text = result?.0?;
        Some(text.lines().next().unwrap_or("").to_owned())
    }

    /// 翻阅历史：只取以开始翻阅时输入的文字为前缀的条目；`older` 为真时取更早的一条。
    fn vim_recall(&mut self, line: &mut Line, older: bool) {
        let history: Vec<String> = match self.bars.prompt_kind {
            PromptKind::VimCommand => shared().command_history().to_vec(),
            _ => shared().search_history().to_vec(),
        };
        let typed = (self.bars.goto_input.as_ref()).map_or(String::new(), |f| f.text().to_owned());
        let (prefix, index) = line.recall.take().unwrap_or((typed, history.len()));
        let fits = |at: &usize| history[*at].starts_with(&prefix);
        let found = if older {
            (0..index.min(history.len())).rev().find(fits)
        } else {
            (index + 1..history.len()).find(fits)
        };
        match found {
            Some(at) => {
                self.vim_line_set(&history[at]);
                line.recall = Some((prefix, at));
            }
            // 越过最新的一条时回到输入的文字。
            None if !older => self.vim_line_set(&prefix),
            None => line.recall = Some((prefix, index)),
        }
    }

    /// 补全命令名、选项名或路径；连续按下时轮换候选，`back` 为真时取上一个。
    ///
    /// 输入条的文字不是上一次补全的结果时重新开始补全。
    fn vim_complete(&mut self, line: &mut Line, back: bool) {
        let text = (self.bars.goto_input.as_ref()).map_or(String::new(), |f| f.text().to_owned());
        let resumed = line.completion.take().filter(|last| last.shown == text);
        let (start, candidates, index) = match resumed {
            Some(last) => {
                let count = last.candidates.len();
                let step = if back { count - 1 } else { 1 };
                (last.start, last.candidates, (last.index + step) % count)
            }
            None => {
                let Some(found) = complete(&text) else {
                    return;
                };
                let partial = text.get(found.start..).unwrap_or("");
                let candidates = match found.kind {
                    Completing::Path => self.vim_paths(partial),
                    Completing::Buffer => {
                        let names = self.editor.documents.iter().map(|doc| doc.name());
                        let mut names: Vec<String> = names
                            .filter(|name| name.starts_with(partial))
                            .map(|name| name.into_owned())
                            .collect();
                        names.sort();
                        names.dedup();
                        names
                    }
                    _ => found.candidates,
                };
                if candidates.is_empty() {
                    return;
                }
                (found.start, candidates, 0)
            }
        };
        let Some(kept) = text.get(..start) else {
            return;
        };
        let shown = format!("{kept}{}", candidates[index]);
        self.vim_line_set(&shown);
        line.completion = Some(Completion {
            start,
            candidates,
            index,
            shown,
        });
    }

    /// 返回命令行中的路径 `path` 所指的位置：相对路径从已打开的目录算起，没有时从当前文档
    /// 所在的目录算起；`~` 是用户目录。
    fn vim_path(&self, path: &str) -> PathBuf {
        if let Some(rest) = path.strip_prefix("~/")
            && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
        {
            return Path::new(&home).join(rest);
        }
        let path = Path::new(path);
        if path.is_absolute() {
            return path.to_path_buf();
        }
        let folder = self.editor.root().map(Path::to_path_buf).or_else(|| {
            let doc = self.editor.active()?;
            Some(doc.path.as_ref()?.parent()?.to_path_buf())
        });
        folder.map_or_else(|| path.to_path_buf(), |folder| folder.join(path))
    }

    /// 返回以 `partial` 开头的路径，按名字排序，取前 [`MAX_PATHS`] 个；目录以 `/` 结尾。
    pub(in crate::app) fn vim_paths(&self, partial: &str) -> Vec<String> {
        let (folder, prefix) = match partial.rfind('/') {
            Some(at) => (&partial[..=at], &partial[at + 1..]),
            None => ("", partial),
        };
        let Ok(entries) =
            winc_backend::fs::read_dir(self.vim_path(if folder.is_empty() { "." } else { folder }))
        else {
            return Vec::new();
        };
        let mut found: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.name.clone().into_string().ok()?;
                let hidden = name.starts_with('.') && !prefix.starts_with('.');
                if !name.starts_with(prefix) || hidden {
                    return None;
                }
                let slash = if winc_backend::fs::is_dir(entry.path()) {
                    "/"
                } else {
                    ""
                };
                Some(format!("{folder}{name}{slash}"))
            })
            .collect();
        found.sort();
        found.truncate(MAX_PATHS);
        found
    }

    /// 执行 Vim 层交给应用的命令。
    pub(super) fn vim_run_app(&mut self, command: AppCommand) {
        let active = self.editor.active().map(|doc| doc.id);
        match command {
            AppCommand::Write { path: None, .. } => self.vim_save(),
            AppCommand::Write {
                path: Some(path),
                force,
            } => self.vim_write_to(&path, force),
            AppCommand::WriteAll => self.vim_save_all(),
            AppCommand::Quit { write, force } => {
                // 差异中先保存可编辑一侧对应的文档，再关闭差异的标签。
                let target = self.vim_document().map(|doc| doc.id);
                if write && target.is_some() && target != active {
                    self.vim_save();
                }
                let dirty = self.editor.active().is_some_and(|doc| doc.is_dirty());
                match (force, active, self.editor.active) {
                    (_, Some(id), _) if write && dirty => self.vim_save_then_close(id),
                    (true, Some(id), _) => {
                        self.discard_tab(id);
                        self.relayout();
                    }
                    (false, _, Some(index)) => self.close_tab(index),
                    _ => {}
                }
            }
            AppCommand::QuitAll { write, .. } => {
                if write {
                    self.vim_save_all();
                }
                self.request_close_window();
            }
            AppCommand::Edit {
                path: Some(path), ..
            }
            | AppCommand::New { path: Some(path) } => {
                let path = self.vim_path(&path);
                self.open_path(&path);
            }
            // 重新读取当前文档没有对应的功能。
            AppCommand::Edit { path: None, .. } => self.flash(t!("editor-vim-unsupported")),
            AppCommand::New { path: None } => self.new_file(),
            AppCommand::BufferStep(step) => {
                let count = self.editor.documents.len();
                for _ in 0..step.unsigned_abs().min(count) {
                    self.cycle_tab(step > 0);
                }
            }
            AppCommand::BufferNumber(number) => self.vim_activate(number.checked_sub(1)),
            AppCommand::BufferName(name) => {
                let documents = self.editor.documents.iter();
                let found = documents
                    .enumerate()
                    .find(|(_, doc)| doc.name().contains(&name));
                self.vim_activate(found.map(|(index, _)| index));
            }
            AppCommand::Only => {
                let others = self.editor.documents.iter().map(|doc| doc.id);
                self.close_tabs(others.filter(|id| Some(*id) != active).collect());
            }
            AppCommand::List(listing) => self.vim_show_list(listing),
            AppCommand::Definition => self.go_to_definition(),
            AppCommand::OpenFile => self.vim_open_file(),
            AppCommand::NoHighlight => {
                if let Some(state) = &mut self.vim {
                    state.highlight = false;
                }
            }
            AppCommand::Focus(focusing) => self.vim_focus(focusing),
            AppCommand::Alternate => self.vim_alternate(),
            AppCommand::GlobalMark { name, line } => self.vim_goto_global_mark(name, line),
            AppCommand::Help => self.vim_show_help(),
            AppCommand::Run(name) => {
                let action = named_action(&name);
                if !action.is_some_and(|action| self.run_action(action)) {
                    self.flash(t!("editor-vim-unsupported"));
                }
            }
        }
        self.request_redraw();
    }

    /// 保存 Vim 层操作的文档：差异中是可编辑一侧对应的文档，其余情况是当前文档。
    fn vim_save(&mut self) {
        let active = self.editor.active().map(|doc| doc.id);
        match self.vim_document().map(|doc| doc.id) {
            Some(id) if Some(id) != active => {
                if !self.confirm_overwrite(id) {
                    self.save_by_id(id);
                }
            }
            _ => self.save_active(),
        }
    }

    /// 把 Vim 层操作的文档写到命令行中的路径 `path`，文档对应的路径不变；没有路径的文档以
    /// 该路径保存。目标上已有文件时只在 `force` 为真时覆盖。
    fn vim_write_to(&mut self, path: &str, force: bool) {
        let to = winc_editor::canonical_path(&self.vim_path(path));
        let Some(doc) = self.vim_document() else {
            return;
        };
        let id = doc.id;
        if doc.path.as_ref() == Some(&to) {
            return self.vim_save();
        }
        if winc_backend::fs::exists(&to) && !force {
            return self.flash(t!("editor-vim-file-exists"));
        }
        if doc.path.is_none() {
            self.save_document(id, Some(to));
            return;
        }
        let name = winc_editor::display_name(&to).into_owned();
        match doc.write_copy(&to) {
            Ok(()) => {
                if let Some(workspace) = &mut self.editor.workspace {
                    workspace.refresh_paths(std::slice::from_ref(&to));
                }
                self.picker_touched(vec![to]);
                self.flash(t!("editor-vim-written", name = name));
            }
            Err(error) => self.notify(t!(
                "workspace-save-failed",
                name = name,
                error = error.to_string()
            )),
        }
    }

    /// 保存文档 `id` 之后关闭其标签；保存没有完成（失败、冲突、取消选择路径）时不关闭。
    fn vim_save_then_close(&mut self, id: DocumentId) {
        if self.closing_pending() {
            return;
        }
        self.wait_for_saves();
        let request = CloseRequest::Tabs(vec![id]);
        self.save_then_close(SavePrompt {
            request,
            dirty: vec![id],
        });
    }

    /// 切换到第 `index` 个文档；不存在时提示。
    fn vim_activate(&mut self, index: Option<usize>) {
        match index.filter(|index| *index < self.editor.documents.len()) {
            Some(index) => self.activate_tab(index),
            None => self.flash(t!("editor-vim-no-buffer")),
        }
    }

    /// 保存全部有路径且有未保存修改的文档。
    fn vim_save_all(&mut self) {
        let dirty: Vec<DocumentId> = (self.editor.documents.iter())
            .filter(|doc| doc.is_dirty() && doc.path.is_some())
            .map(|doc| doc.id)
            .collect();
        for id in dirty {
            self.save_document(id, None);
        }
    }

    /// `gf`：打开光标处的路径；不存在时提示。
    fn vim_open_file(&mut self) {
        let word = self.editor.active().and_then(|doc| {
            let text = doc.text();
            let (line, col) = text.pos_to_line_col(doc.selection.head.min(text.end_pos()));
            let content: Vec<char> = text.line_content(line).slice().chars().collect();
            // 路径由字母、数字与常见的路径符号组成。
            let part = |c: &char| c.is_alphanumeric() || "/\\._-~+".contains(*c);
            let col = col.min(content.len().checked_sub(1)?);
            if !part(&content[col]) {
                return None;
            }
            let start = (0..=col)
                .rev()
                .take_while(|at| part(&content[*at]))
                .last()?;
            let end = (col..content.len())
                .take_while(|at| part(&content[*at]))
                .last()?;
            Some(content[start..=end].iter().collect::<String>())
        });
        let path = word.map(|word| self.vim_path(&word));
        match path.filter(|path| winc_backend::fs::is_file(path)) {
            Some(path) => self.open_path(&path),
            None => self.flash(t!("editor-vim-no-file")),
        }
    }

    /// 设置由应用保存的选项：改动只在本次运行中生效，配置文件中的值记在 [`Kept`] 里。
    pub(super) fn vim_set_option(&mut self, option: HostOption, value: usize) {
        let on = value != 0;
        let tab_width = self.editor.active_tab_width();
        let config = &mut self.config;
        let Some(kept) = self.vim.as_mut().map(|state| &mut state.kept) else {
            return;
        };
        match option {
            HostOption::Number => {
                kept.line_numbers.get_or_insert(config.line_numbers);
                config.line_numbers = Some(on);
                self.relayout();
            }
            HostOption::Wrap => {
                kept.word_wrap.get_or_insert(config.word_wrap);
                config.word_wrap = Some(on);
                self.word_wrap_changed();
            }
            HostOption::List => {
                kept.whitespace.get_or_insert(config.render_whitespace);
                config.render_whitespace = Some(if on {
                    RenderWhitespace::All
                } else {
                    RenderWhitespace::Selection
                });
            }
            HostOption::RelativeNumber => kept.relative = Some(on),
            HostOption::TabStop | HostOption::ShiftWidth | HostOption::ExpandTab => {
                if let Some(doc) = self.editor.active_mut() {
                    match option {
                        HostOption::TabStop => doc.tab_width_override = Some(value),
                        HostOption::ShiftWidth => doc.set_indent_size(value),
                        _ => doc.use_indent(!on, tab_width),
                    }
                }
                self.relayout();
            }
        }
        self.request_redraw();
    }

    /// 把 `:set` 改动过的选项与配置文件中的值对调：保存配置之前与之后各调用一次。
    pub(in crate::app) fn vim_swap_options(&mut self) {
        let config = &mut self.config;
        let Some(kept) = self.vim.as_mut().map(|state| &mut state.kept) else {
            return;
        };
        if let Some(saved) = &mut kept.line_numbers {
            std::mem::swap(saved, &mut config.line_numbers);
        }
        if let Some(saved) = &mut kept.word_wrap {
            std::mem::swap(saved, &mut config.word_wrap);
        }
        if let Some(saved) = &mut kept.whitespace {
            std::mem::swap(saved, &mut config.render_whitespace);
        }
    }

    /// 设置页改动了 `setting`：该项不再保留 `:set` 之前的值。
    pub(in crate::app) fn vim_forget_option(&mut self, setting: crate::layout::Setting) {
        use crate::layout::Setting;
        let Some(kept) = self.vim.as_mut().map(|state| &mut state.kept) else {
            return;
        };
        match setting {
            Setting::LineNumbers => kept.line_numbers = None,
            Setting::WordWrap => kept.word_wrap = None,
            Setting::RenderWhitespace => kept.whitespace = None,
            Setting::VimRelativeNumber => kept.relative = None,
            _ => {}
        }
    }

    /// Vim 模式下是否显示相对行号：`:set` 设置的值优先于配置。
    pub(crate) fn vim_relative(&self) -> bool {
        let Some(state) = &self.vim else {
            return false;
        };
        let configured = self.config.vim_relative_number == Some(true);
        state.kept.relative.unwrap_or(configured)
    }

    /// 记下刚被复制的范围，高亮配置的时长；时长为 0 时不高亮。
    pub(super) fn vim_flash_yank(&mut self, from: usize, to: usize) {
        let wait = self.config.vim_yank_highlight.unwrap_or(YANK_HIGHLIGHT);
        let doc = self.editor.active().map(|doc| doc.id);
        if let (Some(state), Some(doc), true) = (&mut self.vim, doc, wait > 0) {
            let until = Instant::now() + Duration::from_millis(u64::from(wait));
            state.yanked = Some((doc, Selection::new(from, to), until));
        }
    }

    /// 返回文档 `doc` 中仍在高亮的刚被复制的范围。
    pub(crate) fn vim_yanked(&self, doc: DocumentId) -> Option<Selection> {
        let (owner, range, _) = self.vim.as_ref()?.yanked?;
        (owner == doc).then_some(range)
    }

    /// 用列表浮层显示寄存器、标记或已打开的文档。
    fn vim_show_list(&mut self, listing: Listing) {
        let rows: Vec<VimRow> = match listing {
            Listing::Registers => (shared().registers.iter())
                .map(|(name, content)| VimRow {
                    label: format!("\"{name}"),
                    text: preview(&content.text),
                    target: VimTarget::Register(name),
                })
                .collect(),
            Listing::Marks => {
                let Some(doc) = self.editor.active() else {
                    return;
                };
                let text = doc.text();
                let here = doc.modal_marks().into_iter().map(|(name, pos)| {
                    let line = text.line_of(pos);
                    let content = text.line_content(line).slice().to_string();
                    (
                        name,
                        format!("{}  {}", line + 1, preview(content.trim_start())),
                    )
                });
                // 其他文件中的大写标记显示路径与行号。
                (here.chain(self.vim_other_marks()))
                    .map(|(name, text)| VimRow {
                        label: format!("'{name}"),
                        text,
                        target: VimTarget::Mark(name),
                    })
                    .collect()
            }
            Listing::Buffers => (self.editor.documents.iter().enumerate())
                .map(|(index, doc)| {
                    let mark = if doc.is_dirty() { " +" } else { "" };
                    VimRow {
                        label: (index + 1).to_string(),
                        text: format!("{}{mark}", doc.name()),
                        target: VimTarget::Tab(doc.id),
                    }
                })
                .collect(),
        };
        self.finish_name_input();
        self.overlays.picker.places = None;
        self.overlays.picker.branch_rows = None;
        self.overlays.picker.location_rows = None;
        self.overlays.picker.folders = false;
        self.overlays.picker.remote = None;
        self.overlays.picker.vim_list = Some(VimList {
            kind: listing,
            rows,
        });
        self.overlays.picker.open = true;
        self.overlays.picker.doc = self.editor.active().map(|doc| doc.id);
        self.overlays.picker.input.set_text("");
        self.overlays.picker.selected = 0;
        self.overlays.picker.scroll = 0.0;
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 按查询筛选列表：名字或内容中含有查询的各行，不区分大小写。
    pub(in crate::app) fn vim_list_refresh(&mut self) {
        let needle = self.overlays.picker.input.text().to_lowercase();
        let Some(list) = &self.overlays.picker.vim_list else {
            return;
        };
        let fits = |row: &VimRow| {
            row.label.to_lowercase().contains(&needle) || row.text.to_lowercase().contains(&needle)
        };
        let rows = list.rows.iter().enumerate();
        self.overlays.picker.hits =
            (rows.filter(|(_, row)| fits(row)).map(|(index, _)| index)).collect();
    }

    /// 选中列表中的一行：粘贴寄存器、跳转到标记或切换到文档。
    pub(in crate::app) fn vim_list_accept(&mut self) {
        let row = (self.overlays.picker.hits.get(self.overlays.picker.selected))
            .and_then(|index| self.overlays.picker.vim_list.as_ref()?.rows.get(*index));
        let Some(target) = row.map(|row| row.target) else {
            return;
        };
        self.close_picker();
        let keys = match target {
            VimTarget::Register(name) => ['"', name, 'p'],
            VimTarget::Mark(name) => ['`', name, ' '],
            VimTarget::Tab(id) => {
                if let Some(index) = self.editor.index_of(id) {
                    self.activate_tab(index);
                }
                return;
            }
        };
        for c in keys.into_iter().filter(|c| *c != ' ') {
            self.vim_press(winc_vim::Key::char(c));
        }
    }

    /// 测量用：开启 Vim 模式并执行第 `step` 个按键，依次为下移、按词移动、查找、删除一行、
    /// 重复上一次修改、文本对象与替换。
    pub(in crate::app) fn vim_bench_key(&mut self, step: usize) {
        const KEYS: [&str; 9] = ["j", "w", "/", "n", "dd", ".", "ciwx<Esc>", "yi(", ":"];
        if self.vim.is_none() {
            self.config.vim_enabled = Some(true);
            self.vim_apply_config();
        }
        self.focus = Focus::Editor;
        let keys = KEYS[step % KEYS.len()];
        self.vim_feed(keys);
        // 查找的是样本中常见的词：匹配就在近处。
        let line = match keys {
            ":" => "s/e/E/g",
            "/" => "let",
            _ => return,
        };
        self.edit_field(|field| field.insert(line));
        self.submit_prompt();
    }

    /// 返回配置中的映射；`<Leader>` 换成配置的按键。
    pub(super) fn vim_mappings(&self) -> Vec<Mapping> {
        let leader = self.config.vim_leader.as_deref().unwrap_or("\\");
        let leader = notation::write(&notation::parse(leader));
        let expand = |text: &str| {
            text.replace("<Leader>", &leader)
                .replace("<leader>", &leader)
        };
        (self.config.vim_keys.iter())
            .map(|mapping| {
                let mode = match mapping.mode {
                    VimMapMode::Normal => Mode::Normal,
                    VimMapMode::Visual => Mode::Visual,
                    VimMapMode::Insert => Mode::Insert,
                    VimMapMode::Operator => Mode::OperatorPending,
                };
                let to = match named_action(&mapping.to) {
                    Some(_) => Mapped::Command(mapping.to.clone()),
                    None => Mapped::Keys(notation::parse(&expand(&mapping.to))),
                };
                Mapping {
                    mode,
                    keys: notation::parse(&expand(&mapping.keys)),
                    to,
                }
            })
            .collect()
    }

    /// 按键处理之后调用：状态机在等待映射的后续按键时记下超时的时刻。
    pub(super) fn vim_await_mapping(&mut self) {
        let wait = (shared().options.timeoutlen)
            .or(self.config.vim_timeout_len)
            .unwrap_or(TIMEOUT_LEN);
        if let Some(state) = &mut self.vim {
            let waiting = state.vim.awaits_mapping();
            state.map_deadline =
                waiting.then(|| Instant::now() + Duration::from_millis(u64::from(wait)));
        }
    }

    /// 映射等待后续按键超时的时刻已到时通知状态机；返回下一次需要唤醒的时刻。
    pub(in crate::app) fn vim_tick(&mut self, now: Instant) -> Option<Instant> {
        // 复制的高亮到期。
        let state = self.vim.as_mut()?;
        let flash = state.yanked.map(|(_, _, until)| until);
        if flash.is_some_and(|until| now >= until) {
            state.yanked = None;
            self.request_redraw();
        }
        let flash = flash.filter(|until| now < *until);
        let Some(deadline) = self.vim.as_ref()?.map_deadline else {
            return flash;
        };
        if now < deadline {
            return Some(flash.map_or(deadline, |flash| flash.min(deadline)));
        }
        let mut state = self.vim.take()?;
        state.map_deadline = None;
        let lines = (self.editor.active()).map_or(0, |doc| doc.text().line_count());
        let result = self.vim_with(&mut state, |vim, shared, host| vim.timeout(shared, host));
        self.vim = Some(state);
        if let Some(((), messages, deferred)) = result {
            self.vim_after(lines, messages, deferred);
        }
        None
    }
}

/// 返回英文名为 `name` 的命令，不区分大小写。
fn named_action(name: &str) -> Option<Action> {
    let actions = crate::app::palette().iter().copied();
    actions
        .into_iter()
        .find(|action| action.english().eq_ignore_ascii_case(name.trim()))
}
