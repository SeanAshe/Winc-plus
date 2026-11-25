//! 设置中各组选项的名称，以及主题缩略图的顺序。

use super::*;

/// 返回「背景显示方式」中选项 `choice` 的名字。
pub fn background_fit_label(choice: winc_config::BackgroundFit) -> &'static str {
    use winc_config::BackgroundFit::{Fill, Fit, Stretch, Tile};
    match choice {
        Fill => t!("settings-background-fit-fill"),
        Fit => t!("settings-background-fit-fit"),
        Stretch => t!("settings-background-fit-stretch"),
        Tile => t!("settings-background-fit-tile"),
    }
}

/// 返回「Markdown 预览正文宽度」中选项 `choice` 的名字。
pub fn text_width_label(choice: winc_config::TextWidth) -> &'static str {
    use winc_config::TextWidth::{Full, Standard, Wide};
    match choice {
        Standard => t!("settings-text-width-standard"),
        Wide => t!("settings-text-width-wide"),
        Full => t!("settings-text-width-full"),
    }
}

/// 返回「光标移动」设置中选项 `choice` 的名字。
pub fn smooth_caret_label(choice: winc_config::SmoothCaret) -> &'static str {
    use winc_config::SmoothCaret::{Gliding, Off, Snappy};
    match choice {
        Off => t!("settings-smooth-caret-off"),
        Snappy => t!("settings-smooth-caret-snappy"),
        Gliding => t!("settings-smooth-caret-gliding"),
    }
}

/// 返回「自动保存」设置中选项 `choice` 的名字。
pub fn auto_save_label(choice: winc_config::AutoSave) -> &'static str {
    use winc_config::AutoSave::{AfterDelay, Off, OnFocusChange};

    match choice {
        Off => t!("settings-auto-save-off"),
        OnFocusChange => t!("settings-auto-save-focus"),
        AfterDelay => t!("settings-auto-save-delay"),
    }
}

/// 返回「在文件中查找」设置中选项 `choice` 的名字。
pub fn search_style_label(choice: winc_config::SearchStyle) -> &'static str {
    match choice {
        winc_config::SearchStyle::Sidebar => t!("settings-find-in-files-sidebar"),
        winc_config::SearchStyle::Dialog => t!("settings-find-in-files-dialog"),
    }
}

/// 返回「拖入目录」设置中选项 `choice` 的名字。
pub fn drop_folder_label(choice: winc_config::FolderDrop) -> &'static str {
    use winc_config::FolderDrop::{Add, Ask, Open};

    match choice {
        Ask => t!("settings-git-open-worktree-ask"),
        Add => t!("settings-drop-folder-add"),
        Open => t!("settings-drop-folder-open"),
    }
}

/// 返回打开工作树的方式 `choice` 在设置中显示的名称。
pub fn open_worktree_label(choice: winc_config::WorktreeOpen) -> &'static str {
    use winc_config::WorktreeOpen::{Ask, CurrentWindow, NewWindow};
    match choice {
        Ask => t!("settings-git-open-worktree-ask"),
        NewWindow => t!("settings-git-open-worktree-new"),
        CurrentWindow => t!("settings-git-open-worktree-current"),
    }
}

/// 返回拉取方式 `choice` 在设置中显示的名称。
pub fn pull_mode_label(choice: winc_config::PullMode) -> &'static str {
    use winc_config::PullMode::{Config, Merge, Rebase};
    match choice {
        Config => t!("settings-git-pull-config"),
        Merge => t!("settings-git-pull-merge"),
        Rebase => t!("settings-git-pull-rebase"),
    }
}

/// 返回「Vim 剪贴板」设置中选项 `choice` 的名字。
pub fn vim_clipboard_label(choice: winc_config::VimClipboard) -> &'static str {
    use winc_config::VimClipboard::{Always, Register, Yank};
    match choice {
        Register => t!("settings-vim-clipboard-register"),
        Yank => t!("settings-vim-clipboard-yank"),
        Always => t!("settings-vim-clipboard-always"),
    }
}

/// 返回「空白符」设置中选项 `choice` 的名字。
pub fn whitespace_label(choice: winc_config::RenderWhitespace) -> &'static str {
    use winc_config::RenderWhitespace::{All, None, Selection, Trailing};
    match choice {
        None => t!("settings-render-whitespace-none"),
        Selection => t!("settings-render-whitespace-selection"),
        Trailing => t!("settings-render-whitespace-trailing"),
        All => t!("settings-render-whitespace-all"),
    }
}

/// 返回「Meta 键」设置中选项 `choice` 的名字。
pub fn option_as_meta_label(choice: winc_config::OptionAsMeta) -> &'static str {
    use winc_config::OptionAsMeta::{Both, Left, None, Right};
    match choice {
        Left => t!("settings-terminal-option-as-meta-left"),
        Right => t!("settings-terminal-option-as-meta-right"),
        Both => t!("settings-terminal-option-as-meta-both"),
        None => t!("settings-terminal-option-as-meta-none"),
    }
}

/// 「跟随系统」使用当前界面语言，各语言的名称以该语言自身书写，不随界面语言变化。
pub fn locale_label(choice: winc_i18n::LocaleChoice) -> &'static str {
    match choice {
        winc_i18n::LocaleChoice::Auto => t!("settings-language-auto"),
        winc_i18n::LocaleChoice::Fixed(locale) => locale.native_name(),
    }
}

/// 返回一段界面文字的函数；文字随当前界面语言变化，常量表中以此代替字符串。
pub type Label = fn() -> &'static str;

/// 主题缩略图及返回其名称的函数，顺序为从左到右。
pub const THEME_CHOICES: [(ThemeChoice, Label); 3] = [
    (ThemeChoice::Auto, || t!("settings-theme-auto")),
    (ThemeChoice::Light, || t!("settings-theme-light")),
    (ThemeChoice::Dark, || t!("settings-theme-dark")),
];

/// 返回 `Ctrl` 与小写字母 `c` 的组合的显示文字：macOS 上写作 `⌃V`，其他系统上写作 `Ctrl+V`；
/// `c` 不是小写字母时为空字符串。
pub fn ctrl_label(c: char) -> &'static str {
    const KEYS: [&str; 26] = [
        "Ctrl+A", "Ctrl+B", "Ctrl+C", "Ctrl+D", "Ctrl+E", "Ctrl+F", "Ctrl+G", "Ctrl+H", "Ctrl+I",
        "Ctrl+J", "Ctrl+K", "Ctrl+L", "Ctrl+M", "Ctrl+N", "Ctrl+O", "Ctrl+P", "Ctrl+Q", "Ctrl+R",
        "Ctrl+S", "Ctrl+T", "Ctrl+U", "Ctrl+V", "Ctrl+W", "Ctrl+X", "Ctrl+Y", "Ctrl+Z",
    ];
    const MAC: [&str; 26] = [
        "⌃A", "⌃B", "⌃C", "⌃D", "⌃E", "⌃F", "⌃G", "⌃H", "⌃I", "⌃J", "⌃K", "⌃L", "⌃M", "⌃N", "⌃O",
        "⌃P", "⌃Q", "⌃R", "⌃S", "⌃T", "⌃U", "⌃V", "⌃W", "⌃X", "⌃Y", "⌃Z",
    ];
    let table = if cfg!(target_os = "macos") {
        &MAC
    } else {
        &KEYS
    };
    let index = (c as usize).wrapping_sub('a' as usize);
    table.get(index).copied().unwrap_or("")
}

/// 返回「快捷键冲突」一行的值：交给 Vim（`vim` 为真）或编辑器。
pub fn vim_key_owner_label(vim: bool) -> &'static str {
    match vim {
        true => t!("settings-vim-key-vim"),
        false => t!("settings-vim-key-editor"),
    }
}
