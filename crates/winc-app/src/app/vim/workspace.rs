//! Vim 模式与编辑区之外的部分的配合：列表中的移动、焦点在侧栏、编辑区与终端之间的移动、
//! 上一个活动的文档、跨文件的大写标记，以及由命令表生成的帮助页。

use std::fmt::Write as _;
use std::path::PathBuf;

use winc_core::Lines;
use winc_editor::DocumentId;
use winc_i18n::t;
use winc_vim::command::ENTRIES;
use winc_vim::{Focusing, Kind, Mode, Support};
use winit::keyboard::{Key, NamedKey};

use super::shared;
use crate::app::{App, Focus};
use crate::keymap::Action;

/// 焦点所在的位置是否为可以用 Vim 的按键移动的列表：文件树、搜索结果、大纲、改动列表。
fn is_list(focus: Focus) -> bool {
    matches!(
        focus,
        Focus::Tree | Focus::Results | Focus::Outline | Focus::CommitList
    )
}

impl App {
    /// Vim 模式下把列表中的按键 `key` 换成对应的方向键并交给列表处理，返回按键是否已处理。
    ///
    /// `j` `k` 上下移动，`gg` `G` 到首尾，`h` `l` 收起展开，`o` 打开；`Ctrl-W` 之后的 `h` `l`
    /// `j` `k` `w` 移动焦点。其余的按键不处理。
    pub(in crate::app) fn vim_list_key(&mut self, key: &Key, base: &str) -> bool {
        let (focus, modifiers) = (self.focus, self.modifiers);
        let mac = cfg!(target_os = "macos");
        let ctrl_w = winc_vim::Key::ctrl('w');
        let takes_ctrl_w = super::claims(ctrl_w, mac, &self.config.vim_handle_keys);
        let Some(state) = &mut self.vim else {
            return false;
        };
        let prefix = state.list_prefix.take();
        if !is_list(focus) || modifiers.super_key() || modifiers.alt_key() {
            return false;
        }
        let Key::Character(text) = key else {
            return false;
        };
        let typed = text.chars().next().unwrap_or(' ');
        let named = match (prefix, modifiers.control_key(), typed) {
            (Some('w'), _, _) => {
                let focusing = match base {
                    "h" => Some(Focusing::Left),
                    "l" => Some(Focusing::Right),
                    "j" => Some(Focusing::Down),
                    "k" => Some(Focusing::Up),
                    "w" => Some(Focusing::Next),
                    _ => None,
                };
                if let Some(focusing) = focusing {
                    self.vim_focus(focusing);
                }
                return true;
            }
            // `Ctrl-W` 的归属与编辑区相同。
            (_, true, _) if base == "w" && takes_ctrl_w => {
                state.list_prefix = Some('w');
                return true;
            }
            (_, true, _) => return false,
            (Some('g'), _, 'g') => NamedKey::Home,
            (Some('g'), _, _) => return true,
            (None, _, 'g') => {
                state.list_prefix = Some('g');
                return true;
            }
            (None, _, 'j') => NamedKey::ArrowDown,
            (None, _, 'k') => NamedKey::ArrowUp,
            (None, _, 'h') => NamedKey::ArrowLeft,
            (None, _, 'l') => NamedKey::ArrowRight,
            (None, _, 'G') => NamedKey::End,
            (None, _, 'o') => NamedKey::Enter,
            _ => return false,
        };
        let key = Key::Named(named);
        match focus {
            Focus::Tree => self.on_key_tree(&key),
            Focus::Results => self.results_key(&key),
            Focus::Outline => self.outline_key(&key),
            // 历史提交的文件列表借用提交视图的列表。
            _ => {
                if self
                    .with_log_list(|app| app.commit_list_key(&key))
                    .is_none()
                {
                    self.commit_list_key(&key);
                }
            }
        }
        self.request_redraw();
        true
    }

    /// 把焦点移到侧栏、编辑区或终端。
    pub(super) fn vim_focus(&mut self, focusing: Focusing) {
        let editor = self.editor.active.is_some();
        let to_editor = |app: &mut App| {
            if editor {
                app.focus = Focus::Editor;
            }
        };
        let in_terminal = self.focus.in_terminal();
        let in_editor = self.focus == Focus::Editor;
        match focusing {
            Focusing::Left => {
                self.run_action(Action::FocusSidebar);
            }
            Focusing::Right | Focusing::Up => to_editor(self),
            Focusing::Down if !in_terminal => {
                self.run_action(Action::ToggleTerminal);
            }
            Focusing::Down => {}
            // 依次轮换：侧栏、编辑区、已打开的终端。
            Focusing::Next if in_editor && self.terminal.open => {
                self.run_action(Action::ToggleTerminal);
            }
            Focusing::Next if in_editor || in_terminal => {
                self.run_action(Action::FocusSidebar);
            }
            Focusing::Next => to_editor(self),
        }
        self.reset_blink();
        self.request_redraw();
    }

    /// 切换到上一个活动的文档；没有或已关闭时提示。
    pub(super) fn vim_alternate(&mut self) {
        let previous = self.vim.as_ref().and_then(|state| state.previous);
        match previous.and_then(|id| self.editor.index_of(id)) {
            Some(index) => self.activate_tab(index),
            None => self.flash(t!("editor-vim-no-buffer")),
        }
    }

    /// 大写标记 `name` 设在了当前文档的位置 `pos`：记下路径与行列，其他文档中的同名标记作废。
    pub(super) fn vim_set_global_mark(&mut self, name: char, pos: usize) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (id, path) = (doc.id, doc.path.clone());
        let (line, col) = doc.text().pos_to_line_col(pos.min(doc.text().end_pos()));
        for other in &mut self.editor.documents {
            if other.id != id {
                other.clear_modal_mark(name);
            }
        }
        // 没有保存过的文档没有路径，标记只在该文档中有效。
        let Some(path) = path else {
            return;
        };
        self.vim_store_mark(name, path, line, col);
    }

    /// 把大写标记写入状态文件；内容没有变化时不写。
    fn vim_store_mark(&mut self, name: char, path: PathBuf, line: usize, col: usize) {
        let entry = (name, path, line, col);
        if self.state.vim_marks.contains(&entry) {
            return;
        }
        self.update_state(|state| {
            state.vim_marks.retain(|mark| mark.0 != name);
            state.vim_marks.push(entry);
            state.vim_marks.sort();
        });
    }

    /// 把状态文件中属于 Vim 层操作的文档的大写标记设到该文档上，之后随编辑移动；文档尚未
    /// 读入时返回假。
    pub(super) fn vim_attach_marks(&mut self) -> bool {
        let Some(index) = self.vim_target().or(self.editor.active) else {
            return true;
        };
        let doc = &mut self.editor.documents[index];
        if doc.is_pending() {
            return false;
        }
        let Some(path) = doc.path.clone() else {
            return true;
        };
        for (name, at, line, col) in &self.state.vim_marks {
            if *at == path && doc.modal_mark(*name).is_none() {
                let pos = doc.text().line_col_to_pos(*line, *col);
                doc.set_modal_mark(*name, pos);
            }
        }
        true
    }

    /// 各窗口共用的历史为空时，读入状态文件中的命令行历史与查找历史。
    pub(super) fn vim_restore_history(&mut self) {
        let mut shared = shared();
        if shared.search_history().is_empty() && shared.command_history().is_empty() {
            let (searches, commands) = (&self.state.vim_searches, &self.state.vim_commands);
            shared.restore_history(searches.clone(), commands.clone());
        }
    }

    /// 把各文档中大写标记移动后的位置、命令行历史与查找历史写入状态文件；内容没有变化时
    /// 不写。窗口关闭、退出与关闭 Vim 模式之前调用，Vim 模式关闭时不做任何事。
    pub(in crate::app) fn vim_persist(&mut self) {
        if self.vim.is_none() {
            return;
        }
        let ids: Vec<DocumentId> = self.editor.documents.iter().map(|doc| doc.id).collect();
        for id in ids {
            self.vim_store_marks(id);
        }
        let (searches, commands) = {
            let shared = shared();
            let (searches, commands) = (shared.search_history(), shared.command_history());
            (searches.to_vec(), commands.to_vec())
        };
        if self.state.vim_searches != searches || self.state.vim_commands != commands {
            self.update_state(|state| {
                state.vim_searches = searches;
                state.vim_commands = commands;
            });
        }
    }

    /// 文档 `id` 中的大写标记随编辑移动过时，把新的位置写入状态文件；离开或关闭该文档时
    /// 调用。
    pub(in crate::app) fn vim_store_marks(&mut self, id: DocumentId) {
        let Some(doc) = self.editor.documents.iter().find(|doc| doc.id == id) else {
            return;
        };
        let Some(path) = doc.path.clone() else {
            return;
        };
        let text = doc.text();
        let moved: Vec<(char, usize, usize)> = (doc.modal_marks().into_iter())
            .filter(|(name, _)| name.is_ascii_uppercase())
            .map(|(name, pos)| {
                let (line, col) = text.pos_to_line_col(pos);
                (name, line, col)
            })
            .collect();
        for (name, line, col) in moved {
            let owned = (self.state.vim_marks.iter()).any(|mark| mark.0 == name && mark.1 == path);
            if owned {
                self.vim_store_mark(name, path.clone(), line, col);
            }
        }
    }

    /// 跳转到大写标记 `name`：打开所在的文件并把光标放到标记处；`whole_line` 为真时落在所在
    /// 行的第一个非空白字符上。
    pub(super) fn vim_goto_global_mark(&mut self, name: char, whole_line: bool) {
        let found = self.state.vim_marks.iter().find(|mark| mark.0 == name);
        let Some((_, path, line, col)) = found.cloned() else {
            return self.flash(t!("editor-vim-no-mark", name = name.to_string()));
        };
        let open = (self.editor.documents.iter()).position(|doc| doc.path.as_ref() == Some(&path));
        match open {
            Some(index) => self.activate_tab(index),
            None if path.is_file() => self.open_path(&path),
            None => return self.flash(t!("editor-vim-no-file")),
        }
        self.navigation.mark_jump();
        self.focus = Focus::Editor;
        self.when_loaded(move |app| {
            let Some(doc) = app.editor.active_mut() else {
                return;
            };
            let text = doc.text();
            let marked = text.line_col_to_pos(line, col);
            let row = text.line_of(marked);
            let pos = if whole_line {
                let indent = text.line_content(row).slice().to_string();
                let blanks = indent.chars().take_while(|c| matches!(c, ' ' | '\t'));
                text.line_start(row) + blanks.count()
            } else {
                marked
            };
            // 标记之后随该文档的编辑移动。
            doc.set_modal_mark(name, marked);
            app.scroll_to_line(row, 1.0 / 3.0, true);
            app.select(pos, pos);
        });
    }

    /// 返回不在当前文档中的大写标记：名字、路径与行号，供 `:marks` 列出。
    pub(super) fn vim_other_marks(&self) -> Vec<(char, String)> {
        let current = self.editor.active().and_then(|doc| doc.path.as_ref());
        (self.state.vim_marks.iter())
            .filter(|mark| Some(&mark.1) != current)
            .map(|(name, path, line, _)| (*name, format!("{}:{}", path.display(), line + 1)))
            .collect()
    }

    /// `:help`：在只读标签中显示由命令表生成的命令一览。
    pub(super) fn vim_show_help(&mut self) {
        let text = help_text();
        self.editor.show_output(t!("editor-vim-help-title"), &text);
        self.focus = Focus::Editor;
        self.relayout();
        self.ensure_caret_visible();
    }
}

/// 返回命令一览的文本：命令表中的每条记录一行，其后是命令行的命令与 `:set` 的选项。
fn help_text() -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}\n", t!("editor-vim-help-title"));
    let _ = writeln!(out, "{}\n", t!("editor-vim-help-modes"));
    let header = (
        t!("editor-vim-help-keys"),
        t!("editor-vim-help-mode"),
        t!("editor-vim-help-kind"),
        t!("editor-vim-help-status"),
    );
    let _ = writeln!(
        out,
        "{:<14}{:<10}{:<14}{}",
        header.0, header.1, header.2, header.3
    );
    for entry in ENTRIES {
        let modes: String = [
            (Mode::Normal, "N "),
            (Mode::Visual, "V "),
            (Mode::OperatorPending, "O "),
            (Mode::Insert, "I "),
        ]
        .iter()
        .filter(|(mode, _)| entry.modes.contains(*mode))
        .map(|(_, letter)| *letter)
        .collect();
        let (keys, kind) = match entry.kind {
            Kind::Motion(_) => (entry.keys, t!("editor-vim-help-motion")),
            Kind::TextObject(_) => (entry.keys, t!("editor-vim-help-object")),
            Kind::Register | Kind::Force(_) => (entry.keys, t!("editor-vim-help-prefix")),
            // 成对符号在等待动作时接在 `y` `d` `c` 之后。
            Kind::Surround if entry.modes.contains(Mode::OperatorPending) => {
                ("ys ds cs", t!("editor-vim-help-operator"))
            }
            Kind::Operator(_)
            | Kind::LineOperator(_)
            | Kind::WideOperator(..)
            | Kind::SmallDelete
            | Kind::Synonym(..)
            | Kind::Surround => (entry.keys, t!("editor-vim-help-operator")),
            Kind::Action(_) => (entry.keys, t!("editor-vim-help-command")),
        };
        let status = match entry.support {
            Support::Supported => t!("editor-vim-help-supported"),
            _ => t!("editor-vim-help-unsupported"),
        };
        let _ = writeln!(out, "{keys:<14}{modes:<10}{kind:<14}{status}");
    }
    let _ = writeln!(out, "\n{}\n", t!("editor-vim-help-ex"));
    for (name, min) in winc_vim::ex::command_names() {
        let (short, rest) = name.split_at(min.min(name.len()));
        let _ = if rest.is_empty() {
            writeln!(out, ":{short}")
        } else {
            writeln!(out, ":{short}[{rest}]")
        };
    }
    let _ = writeln!(out, ":>  :<  :&  :&&  :#");
    let _ = writeln!(out, "\n{}\n", t!("editor-vim-help-options"));
    for (name, short) in winc_vim::option_list() {
        let _ = if name == short {
            writeln!(out, "{name}")
        } else {
            writeln!(out, "{name}  ({short})")
        };
    }
    out
}
