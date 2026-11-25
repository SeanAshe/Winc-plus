//! `Ctrl` 键归 Vim 层还是归编辑器：列出两边都要用的键，查看和修改每个键的归属。
//!
//! 归属记在配置的 `vim.handleKeys` 中，只记与默认规则（见 [`super::claims`]）不同的键；设置页中
//! 「快捷键冲突」一组的每一行对应一个键。

use winc_i18n::t;

use crate::app::{App, NoticeAction, NoticeKind};
use crate::keymap::{Binding, KeyCode, Mods, Stroke};

/// 返回 `Ctrl` 与字符 `c` 的组合在配置 `vim.handleKeys` 中的名字，如 `ctrl+v`。
fn config_name(c: char) -> String {
    format!("ctrl+{c}")
}

/// 是否在 macOS 上：那里的快捷键用 `⌘`，`Ctrl` 键默认全部归 Vim 层。
fn mac() -> bool {
    cfg!(target_os = "macos")
}

/// Vim 层是否有以 `Ctrl` 与字符 `c` 开头的命令（任一模式）。
fn vim_defines(c: char) -> bool {
    let notation = winc_vim::notation::write(&[winc_vim::Key::ctrl(c)]);
    (winc_vim::command::ENTRIES.iter()).any(|entry| entry.keys.starts_with(&notation))
}

/// 返回 `Ctrl` 与字符 `c` 在编辑区对应的第一条绑定；没有时返回 `None`。
fn editor_binding(c: char) -> Option<Binding> {
    crate::keymap::editor_binding(Stroke {
        key: KeyCode::Char(c),
        mods: Mods::CTRL,
    })
}

/// 没有改写时 `Ctrl` 与字符 `c` 是否归 Vim 层。
fn vim_by_default(c: char) -> bool {
    super::claims(winc_vim::Key::ctrl(c), mac(), &[])
}

impl App {
    /// 返回两边都要用的 `Ctrl` 键（按字母顺序）：Vim 层有以它开头的命令，当前键位表在编辑区也给
    /// 它绑定了命令。Vim 模式关闭时为空。
    pub(crate) fn vim_key_conflicts(&self) -> Vec<char> {
        if self.vim.is_none() {
            return Vec::new();
        }
        ('a'..='z')
            .filter(|&c| vim_defines(c) && editor_binding(c).is_some())
            .collect()
    }

    /// 重新算出设置页中列出的冲突键：Vim 模式开关、键位表变化之后调用。
    pub(in crate::app) fn refresh_vim_key_rows(&mut self) {
        self.settings.listed.vim_keys = self.vim_key_conflicts();
    }

    /// `Ctrl` 与字符 `c` 当前是否归 Vim 层：先看 `vim.handleKeys`，其次按默认规则。
    pub(crate) fn vim_owns_key(&self, c: char) -> bool {
        super::claims(winc_vim::Key::ctrl(c), mac(), &self.config.vim_handle_keys)
    }

    /// `Ctrl` 与字符 `c` 的归属是否改过：`vim.handleKeys` 中有它，并且与默认不同。
    pub(crate) fn vim_key_changed(&self, c: char) -> bool {
        self.vim_owns_key(c) != vim_by_default(c)
    }

    /// 把 `Ctrl` 与字符 `c` 交给 Vim 层（`vim` 为真）或编辑器，写入配置文件；与默认相同时从
    /// `vim.handleKeys` 中去掉这一项。
    pub(in crate::app) fn set_vim_key_owner(&mut self, c: char, vim: bool) {
        let name = config_name(c);
        let keys = &mut self.config.vim_handle_keys;
        keys.retain(|(key, _)| *key != name);
        if vim != vim_by_default(c) {
            keys.push((name, vim));
            keys.sort();
        }
        self.save_config();
        self.request_redraw();
    }

    /// 把 `Ctrl` 与字符 `c` 的归属恢复为默认。
    pub(in crate::app) fn reset_vim_key(&mut self, c: char) {
        self.set_vim_key_owner(c, vim_by_default(c));
    }

    /// Vim 层刚处理了按键 `key`：它是编辑区也在用的 `Ctrl` 键、归属没有改写过，并且从没提示过时，
    /// 提示这一次按键由 Vim 处理、没有执行编辑器的命令，附「打开设置」。同一个键在别的模式下
    /// 可能仍由编辑器处理（插入模式的 `Ctrl+V`），因此只说这一次。
    ///
    /// 只提示一次：之后的冲突在设置的「快捷键冲突」中查看。
    pub(in crate::app) fn vim_notice_key(&mut self, key: winc_vim::Key) {
        if self.state.vim_keys_noticed {
            return;
        }
        let winc_vim::Code::Char(c) = key.code() else {
            return;
        };
        if key.mods() != winc_vim::Mods::CTRL || !c.is_ascii_lowercase() {
            return;
        }
        let configured =
            (self.config.vim_handle_keys.iter()).any(|(name, _)| *name == config_name(c));
        let Some(binding) = editor_binding(c).filter(|_| !configured) else {
            return;
        };
        self.update_state(|state| state.vim_keys_noticed = true);
        let key = crate::layout::ctrl_label(c);
        let action = binding.action.title();
        let message = t!("editor-vim-key-notice", key = key, action = action);
        self.notify_with(NoticeKind::Info, message, NoticeAction::VimKey(c));
    }
}
