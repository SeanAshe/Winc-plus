//! 筛选：按文字、分支、作者、日期、路径，以及一个文件的历史。

use winc_core::Lines;

use super::*;
use crate::app::input::context_menu::MenuEntry;

use crate::app::scroll::set_scroll_line;

/// 标题栏的一个筛选下拉；按声明的顺序从左到右排。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LogPill {
    /// 分支。
    Branch,
    /// 作者。
    Author,
    /// 日期。
    Date,
    /// 路径；文件历史没有。
    Path,
}

impl LogPill {
    /// 全部筛选下拉。
    pub(crate) const ALL: [LogPill; 4] = [
        LogPill::Branch,
        LogPill::Author,
        LogPill::Date,
        LogPill::Path,
    ];

    /// 点它时的目标。
    pub(crate) fn target(self) -> LogTarget {
        match self {
            LogPill::Branch => LogTarget::Branch,
            LogPill::Author => LogTarget::Author,
            LogPill::Date => LogTarget::Date,
            LogPill::Path => LogTarget::Path,
        }
    }
}

/// 日期菜单里「最近几天」的几项（天）：最近 24 小时、一周、30 天、一年。自选日期区间要用日历
/// 选择，这里以这几项常用的范围代替。
const LOG_SINCE_DAYS: [u32; 4] = [1, 7, 30, 365];

/// 「最近 `days` 天」在日期菜单和标题栏里的写法。
pub(crate) fn since_label(days: u32) -> String {
    match days {
        1 => t!("menu-log-last-day").to_owned(),
        365 => t!("menu-log-last-year").to_owned(),
        _ => t!("menu-log-last-days", count = days),
    }
}

impl App {
    /// 标题栏「日期」「路径」「作者」下拉弹出的菜单：最上面是不筛选的一项，下面是各个选项；路径
    /// 菜单最后是「其他路径…」。
    pub(in crate::app) fn log_filter_menu(&self, target: &MenuTarget) -> Vec<MenuEntry> {
        let item = |command| MenuEntry::Item(command, true);
        let (all, rest): (MenuCommand, Vec<MenuCommand>) = match target {
            MenuTarget::LogDate => (
                MenuCommand::LogSince(0),
                LOG_SINCE_DAYS.map(MenuCommand::LogSince).to_vec(),
            ),
            MenuTarget::LogPath => {
                let paths = 1..=self.git.log.paths.len();
                (
                    MenuCommand::LogPath(0),
                    paths.map(MenuCommand::LogPath).collect(),
                )
            }
            _ => {
                let authors = 1..=self.git.log.authors.len();
                (
                    MenuCommand::LogAuthor(0),
                    authors.map(MenuCommand::LogAuthor).collect(),
                )
            }
        };
        let mut menu = vec![item(all)];
        // 作者菜单：读到了自己的邮箱时，「全部作者」下面是「我」。
        if *target == MenuTarget::LogAuthor && !self.git.my_email.is_empty() {
            menu.push(item(MenuCommand::LogAuthorMe));
        }
        menu.push(MenuEntry::Separator);
        let listed = !rest.is_empty();
        menu.extend(rest.into_iter().map(item));
        if *target == MenuTarget::LogPath {
            if listed {
                menu.push(MenuEntry::Separator);
            }
            menu.push(item(MenuCommand::LogPathOther));
        }
        menu
    }

    /// 筛选菜单里一个选项（作者、最近几天、路径）的文字。
    pub(in crate::app) fn log_filter_label(&self, command: MenuCommand) -> String {
        let listed = |list: &[String], i: usize| list.get(i - 1).cloned().unwrap_or_default();
        match command {
            MenuCommand::LogAuthor(i) => listed(&self.git.log.authors, i),
            MenuCommand::LogPath(i) => listed(&self.git.log.paths, i),
            MenuCommand::LogSince(days) => since_label(days),
            _ => String::new(),
        }
    }

    /// 日期、路径菜单里的一项是否打勾（是当前日志的筛选条件）；不是这两种菜单项时为 `None`。
    pub(in crate::app) fn log_filter_checked(&self, command: MenuCommand) -> Option<bool> {
        let query = self
            .editor
            .active()
            .and_then(|d| Some(&d.log.as_deref()?.query));
        match command {
            MenuCommand::LogSince(days) => {
                Some(query.is_some_and(|q| q.since_days.unwrap_or(0) == days))
            }
            MenuCommand::LogAuthorMe => {
                let me = self.git.my_email.as_str();
                Some(query.is_some_and(|q| q.authors.iter().any(|a| a == me)))
            }
            MenuCommand::LogPath(i) => Some(query.is_some_and(|q| match i.checked_sub(1) {
                None => q.paths.is_empty(),
                Some(i) => (self.git.log.paths.get(i)).is_some_and(|p| q.paths.contains(p)),
            })),
            _ => None,
        }
    }

    /// 路径菜单列出的路径：仓库根目录下的各个目录（不含隐藏的，按文件树的顺序），以及已经选上
    /// 的其他路径。
    pub(super) fn log_path_choices(&self) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        if let Some(root) = self.git.root.as_deref()
            && let Ok(entries) = winc_editor::list_dir(root)
        {
            let dirs = entries
                .into_iter()
                .filter(|e| e.kind == winc_editor::EntryKind::Dir);
            paths.extend(dirs.map(|e| e.name).filter(|n| !n.starts_with('.')));
        }
        let chosen = self.editor.active().and_then(|d| d.log.as_deref());
        for path in chosen.iter().flat_map(|v| &v.query.paths) {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        paths
    }

    /// 路径筛选里加上或去掉 `path`。
    pub(in crate::app) fn toggle_log_path(&mut self, path: String) {
        self.change_log_query(|q| match q.paths.iter().position(|p| *p == path) {
            Some(at) => _ = q.paths.remove(at),
            None => q.paths.push(path),
        });
    }

    /// 打开输入其他路径的对话框（路径菜单的「其他路径…」）。
    pub(in crate::app) fn open_log_path_dialog(&mut self) {
        use crate::app::git::form::FormKind;
        self.show_form_dialog(
            FormKind::LogPath,
            TextField::default(),
            crate::layout::DialogPart::Branch,
        );
    }

    /// 作者筛选里加上或去掉 `author`。
    pub(in crate::app) fn toggle_log_author(&mut self, author: String) {
        self.change_log_query(|q| match q.authors.iter().position(|a| *a == author) {
            Some(at) => _ = q.authors.remove(at),
            None => q.authors.push(author),
        });
    }

    /// 当前日志只按一个分支筛选时的那个分支；没按分支筛选、或选了几个分支时为 `None`。
    pub(crate) fn log_branch_filter(&self) -> Option<&str> {
        match self
            .editor
            .active()?
            .log
            .as_ref()?
            .query
            .branches
            .as_slice()
        {
            [one] => Some(one.as_str()),
            _ => None,
        }
    }

    /// 打开文件 `path` 的历史：一个只看改过这个文件的提交的日志标签（跟着改名往前找）；已经
    /// 开着就切过去。
    pub(in crate::app) fn git_file_history(&mut self, path: &Path) {
        self.open_history(path, None);
    }

    /// 当前文件中选中的几行（没有选区时是光标所在的一行）的历史（命令「Git：显示所选代码的
    /// 历史」）：只看改过这几行的提交。
    ///
    /// `git log -L` 按 `HEAD` 中的行号查找：未提交的改动先按与 `HEAD` 的差异换算，新加的行
    /// 换成它所在位置在 `HEAD` 中的那一行。
    pub(in crate::app) fn git_selection_history(&mut self) {
        self.fresh_line_changes();
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(path) = doc.path.clone() else {
            self.flash(t!("git-no-file-in-tab"));
            return;
        };
        let text = doc.text();
        let (from, to) = (doc.selection.from(), doc.selection.to());
        let first = text.line_of(from);
        // 选区结束在下一行行首时，那一行没有被选中。
        let last = match text.line_of(to) {
            line if line > first && text.line_start(line) == to => line - 1,
            line => line,
        };
        let hunks = self.git.changes.hunks(doc.id);
        let range = (head_line(hunks, first) + 1, head_line(hunks, last) + 1);
        self.open_history(&path, Some(range));
    }

    /// 打开文件 `path` 的历史标签；`lines` 是只看的行范围（见 [`LogQuery::lines`]）。
    fn open_history(&mut self, path: &Path, lines: Option<(usize, usize)>) {
        let Some(root) = self.git.root.clone() else {
            self.flash(t!("git-not-a-repository"));
            return;
        };
        // 路径可能经过符号链接（macOS 的 `/var` 就是 `/private/var`）。
        let path = match winc_config::files::canonicalize(path) {
            Ok(real) if real.starts_with(&root) => real,
            _ if path.starts_with(&root) => path.to_path_buf(),
            _ => {
                self.flash(t!("git-file-outside-repository"));
                return;
            }
        };
        let title = match lines {
            Some((start, end)) => {
                let name = winc_editor::display_name(&path);
                let (start, end) = (start.to_string(), end.to_string());
                t!(
                    "git-lines-history-title",
                    name = name,
                    start = start,
                    end = end
                )
            }
            None => history_title(&path),
        };
        let (index, new) = self.editor.show_log(&title);
        let id = self.editor.documents[index].id;
        if new {
            if let Some(view) = self.editor.documents[index].log.as_deref_mut() {
                view.query.path = Some(path);
                view.query.lines = lines;
            }
            self.load_log(id);
        }
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// 当前编辑的文件的历史（命令「Git：显示当前文件的历史」）。
    pub(in crate::app) fn git_current_file_history(&mut self) {
        match self.editor.active().and_then(|d| d.path.clone()) {
            Some(path) => self.git_file_history(&path),
            None => self.flash(t!("git-no-file-in-tab")),
        }
    }

    /// 筛选输入框里的文字变了：当前日志按新的文字从头再取。
    pub(in crate::app) fn log_filter_changed(&mut self) {
        let text = self.git.log.filter.text().to_owned();
        // 改了筛选文字即离开转到的提交，不再固定它。
        self.change_log_query(|q| {
            q.text = text;
            q.pin = None;
        });
    }

    /// 改当前日志的筛选条件，从头再取。
    pub(in crate::app) fn change_log_query(&mut self, change: impl FnOnce(&mut LogQuery)) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let id = doc.id;
        let Some(view) = doc.log.as_deref_mut() else {
            return;
        };
        let before = view.query.clone();
        change(&mut view.query);
        if view.query != before {
            // 选着的提交留着：新的结果里还有它就继续选它，详情一栏不在每次输入时先清空。
            view.reset();
            if let Some(doc) = self.editor.active_mut() {
                set_scroll_line(doc, 0.0);
            }
            self.load_log(id);
        }
        self.request_redraw();
    }

    /// 当前日志标签的筛选输入框里该有的文字（切换标签时换过来）。
    pub(crate) fn sync_log_filter(&mut self) {
        if self.focus == Focus::LogFilter {
            return;
        }
        let text = self
            .editor
            .active()
            .and_then(|d| d.log.as_deref())
            .map(|v| v.query.text.clone());
        if let Some(text) = text
            && self.git.log.filter.text() != text
        {
            self.git.log.filter.set_text(&text);
        }
    }

    /// 日志筛选输入框里的按键：`Esc` 有文字时清空、没有时回到列表，回车、`↓` 回到列表，
    /// 其余交给输入框。
    pub(in crate::app) fn on_key_log_filter(
        &mut self,
        event: &winit::event::KeyEvent,
        base: &str,
        cmd: bool,
    ) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) if !self.git.log.filter.text().is_empty() => {
                self.edit_field(|f| {
                    f.set_text("");
                    true
                });
            }
            Key::Named(NamedKey::Escape | NamedKey::Enter | NamedKey::ArrowDown) => {
                self.focus = Focus::Editor;
                self.request_redraw();
            }
            _ => self.on_key_field(event, base, cmd),
        }
    }

    /// 作者菜单里列的作者：已经取回来的提交里的作者，按提交数从多到少，最多 30 个
    /// （经验值：再长菜单就超出窗口高度）。
    ///
    /// 已经按作者筛选时取回来的只剩这几位的提交：沿用这个标签上次弹出时的名单，才能再加别的
    /// 作者。
    pub(super) fn log_authors(&self) -> Vec<String> {
        let Some(doc) = self.editor.active() else {
            return Vec::new();
        };
        let Some(view) = doc.log.as_deref() else {
            return Vec::new();
        };
        let same = self.git.log.authors_of == Some(doc.id);
        if !view.query.authors.is_empty() && same && !self.git.log.authors.is_empty() {
            let mut list = self.git.log.authors.clone();
            for author in &view.query.authors {
                if !list.contains(author) && *author != self.git.my_email {
                    list.insert(0, author.clone());
                }
            }
            return list;
        }
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for commit in &view.commits {
            *counts.entry(commit.author.as_str()).or_default() += 1;
        }
        let mut authors: Vec<(&str, usize)> = counts.into_iter().collect();
        authors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        // 当前按作者筛选时，列表里只剩这几位作者：也列出来，以便打勾、取消。
        let mut list: Vec<String> = authors
            .into_iter()
            .take(30)
            .map(|(a, _)| a.to_owned())
            .collect();
        for author in view.query.authors.iter().rev() {
            if !list.contains(author) && *author != self.git.my_email {
                list.insert(0, author.clone());
            }
        }
        list
    }
}

/// 把当前文本的第 `line` 行（从 0 起）换成 `HEAD` 中对应的行：`hunks` 是从 `HEAD` 到当前文本的
/// 改动。改动之后的行按前面增减的行数平移；落在改动里的行换成这段改动在 `HEAD` 中的开头，
/// 那段在 `HEAD` 中没有内容（新加的行）时是它前面的一行。
pub(crate) fn head_line(hunks: &[winc_core::diff::LineHunk], line: usize) -> usize {
    let mut shift: isize = 0;
    for hunk in hunks {
        if line < hunk.new.start {
            break;
        }
        if line < hunk.new.end {
            return match hunk.old.is_empty() {
                true => hunk.old.start.saturating_sub(1),
                false => hunk.old.start,
            };
        }
        shift += hunk.old.len() as isize - hunk.new.len() as isize;
    }
    line.saturating_add_signed(shift)
}
