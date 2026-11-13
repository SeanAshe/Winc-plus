//! 终端里的查找（⌘F）：在回滚缓冲加屏幕里找文字。
//!
//! 打开查找条时向辅助进程要整个回滚缓冲的文字（辅助进程不带正则库，查找在主程序这边做），
//! 只留文字、不留样式，关掉查找条时释放。查找的词每改一次就从最下面一行往上重新找，每帧
//! 只找一个时间片（[`SLICE`]），回滚缓冲很长时输入框也不阻塞。区分大小写、全字匹配和正则的语义与
//! 编辑区的查找相同（[`TextSearch`]）。软换行的各行拼接后查找；查找条打开期间有新输出时重新查找，
//! 当前匹配和滚动位置保持不变。
//!
//! 查找条浮在内容区右上角。焦点离开时查找条保持打开，× 或查找条内的 Esc 关闭。

use std::time::{Duration, Instant};

use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use winc_editor::TextSearch;
use winc_pty::protocol::Line;
use winc_render::Rect;

use super::tab::{Point, Selection, Unit, col_at, logical_text};
use super::{App, Focus, Request, header_height};
use crate::field::TextField;
use crate::theme::TAB_HEIGHT;

/// 每次查找最多连续占用的时间（8 ms），与编辑区查找大文件的时间片相同。
const SLICE: Duration = Duration::from_millis(8);
/// 查找条的宽度（逻辑像素）：选项和按钮之外可容纳计数和约 25 个字符的查找词；面板更窄时同步收窄。
const FIND_W: f32 = 400.0;
/// 显示三个选项所需的最小宽度（逻辑像素）：低于该值时输入框不足约 10 个字符，只保留输入框和按钮，
/// 已选的选项仍然生效。
pub(crate) const FIND_OPTIONS_MIN: f32 = 340.0;
/// 有新输出时重新查找的最短间隔（250 ms）：大量输出时结果无需逐帧更新，输出停止后的延迟不超过该值。
const FIND_REFRESH: Duration = Duration::from_millis(250);
/// 查找条与面板右边的距离（逻辑像素），为滚动条留出位置。
const FIND_MARGIN: f32 = 12.0;

/// 查找条上可点击的部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindPart {
    /// 按钮之外的区域：点击后焦点回到输入框，事件不传给下方的终端。
    Bar,
    /// 上一处匹配。
    Previous,
    /// 下一处匹配。
    Next,
    /// 关闭查找条。
    Close,
    /// 「区分大小写」选项。
    Case,
    /// 「全字匹配」选项。
    Word,
    /// 「使用正则表达式」选项。
    Regex,
}

/// 查找的选项。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FindOptions {
    /// 区分大小写。
    pub case_sensitive: bool,
    /// 全字匹配。
    pub whole_word: bool,
    /// 使用正则表达式。
    pub regex: bool,
}

/// 查找条的状态。
#[derive(Default)]
pub(crate) struct TermFind {
    /// 查找的词。
    pub field: TextField,
    /// 在哪个标签里找。
    pub tab: u16,
    /// 选项。
    pub options: FindOptions,
    /// 查找词的编译结果；查找词为空或正则有误时为 `None`。
    matcher: Option<TextSearch>,
    /// 正则表达式有误。
    pub invalid: bool,
    /// 从辅助进程取回的回滚缓冲：（首行行号，各行文字和是否软换行到下一行），按行号升序。
    chunks: Vec<(u64, Vec<(String, bool)>)>,
    /// 回滚缓冲取回来了没有（没回来时先只找屏幕）。
    pub loaded: bool,
    /// 所有匹配，按位置从上到下：（起点，终点）。
    pub matches: Vec<(Point, Point)>,
    /// 当前的那一处。
    pub current: Option<usize>,
    /// 还没找完时，下一个时间片从这一行的上一行往上找。
    scan: Option<u64>,
    /// 有新输出尚未重新查找。
    stale: bool,
    /// 上次因新输出重新查找的时刻。
    refreshed: Option<Instant>,
    /// 正在读取新进入回滚缓冲的行。
    fetching: bool,
    /// 因新输出重新查找时的中间结果：全部完成后才替换 `matches`，已有高亮不会闪烁。
    rescan: Option<Vec<(Point, Point)>>,
}

impl App {
    /// 查找条的位置：和内容区第一行垂直居中，高度与标签相同；没有打开时为 `None`。
    pub(crate) fn terminal_find_bar(&self) -> Option<Rect> {
        self.terminal.find.as_ref()?;
        let card = self.layout.terminal;
        if card.w <= 0.0 || card.h <= 0.0 {
            return None;
        }
        let row = card.y + header_height(card);
        let top = row + (self.fonts.terminal_line_height - TAB_HEIGHT) / 2.0;
        let w = FIND_W.min(card.w - FIND_MARGIN * 2.0).max(0.0);
        Some(Rect::new(
            card.right() - FIND_MARGIN - w,
            top,
            w,
            TAB_HEIGHT,
        ))
    }

    /// 处理查找条上的点击。焦点保留在输入框，之后可以继续按 ↩ 跳转。
    pub(in crate::app) fn press_terminal_find(&mut self, part: FindPart) {
        if part == FindPart::Close {
            self.close_terminal_find();
            return;
        }
        self.focus = Focus::TerminalFind;
        let Some(options) = self.terminal.find.as_mut().map(|find| &mut find.options) else {
            return;
        };
        match part {
            FindPart::Previous => self.terminal_find_step(true),
            FindPart::Next => self.terminal_find_step(false),
            FindPart::Case => {
                options.case_sensitive = !options.case_sensitive;
                self.terminal_find_changed();
            }
            FindPart::Word => {
                options.whole_word = !options.whole_word;
                self.terminal_find_changed();
            }
            FindPart::Regex => {
                options.regex = !options.regex;
                self.terminal_find_changed();
            }
            FindPart::Bar | FindPart::Close => {}
        }
        self.request_redraw();
    }

    /// 打开查找条（⌘F）；已经开着时把焦点放回输入框、全选。
    pub(in crate::app) fn open_terminal_find(&mut self) {
        let t = &mut self.terminal;
        let Some(tab) = t.tabs.get(t.active) else {
            return;
        };
        let (id, history) = (tab.id, tab.history);
        if t.find.as_ref().is_none_or(|f| f.tab != id) {
            t.find = Some(TermFind {
                tab: id,
                ..TermFind::default()
            });
            t.request(id, history, Request::Find);
            // 回滚缓冲是空的：没有要等的。
            if let Some(find) = &mut t.find {
                find.loaded = history.0 >= history.1;
            }
        }
        if let Some(find) = &mut t.find {
            find.field.select_all();
        }
        self.focus = Focus::TerminalFind;
        self.terminal_find_changed();
        self.request_redraw();
    }

    /// 关掉查找条，焦点回终端；保留选中的匹配（可以直接 ⌘C）。
    pub(in crate::app) fn close_terminal_find(&mut self) {
        self.terminal.find = None;
        // 还没回来的各段不再使用：马上再 ⌘F 时混进新的查找，每处匹配算两遍。
        self.terminal
            .requests
            .retain(|r| r.2 != super::Request::Find);
        self.focus = Focus::Terminal;
        self.request_redraw();
    }

    /// 回滚缓冲的一段文字取回来了（`done`：最后一段）。
    pub(in crate::app) fn terminal_find_loaded(
        &mut self,
        tab: u16,
        start: u64,
        lines: Vec<Line>,
        done: bool,
    ) {
        if let Some(find) = self.terminal.find.as_mut().filter(|f| f.tab == tab) {
            let lines = lines
                .iter()
                .map(|line| (logical_text(line).to_owned(), line.wrapped));
            find.chunks.push((start, lines.collect()));
            find.loaded = done;
            match (done, find.fetching) {
                (true, true) => {
                    find.fetching = false;
                    find.stale = true;
                    find.refreshed = None;
                }
                (true, false) => self.terminal_find_changed(),
                (false, _) => {}
            }
        }
    }

    /// 标签的行号全部重来了（改尺寸重排、⌘K）：查找要重新取回滚缓冲，F8 记的位置作废。
    pub(in crate::app) fn terminal_renumbered(&mut self, id: u16) {
        let t = &mut self.terminal;
        if t.errors.as_ref().is_some_and(|e| e.tab == id) {
            t.errors = None;
        }
        let Some(history) = t
            .tabs
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.history)
        else {
            return;
        };
        if let Some(find) = t.find.as_mut().filter(|f| f.tab == id) {
            find.chunks.clear();
            find.fetching = false;
            find.loaded = history.0 >= history.1;
            t.requests
                .retain(|&(_, tab, purpose)| (tab, purpose) != (id, Request::Find));
            t.request(id, history, Request::Find);
            self.terminal_find_changed();
        }
    }

    /// 查找的词变了（或者文字刚取回来）：清掉旧的匹配，从最下面一行往上重新找。
    ///
    /// 这里先找一个时间片，没找完的部分在之后的帧里由 [`App::continue_terminal_find`] 继续。
    pub(in crate::app) fn terminal_find_changed(&mut self) {
        self.restart_terminal_find();
        self.continue_terminal_find(SLICE);
    }

    /// 清掉旧的匹配，把查找的位置放回最下面一行之后，不开始找。
    pub(in crate::app) fn restart_terminal_find(&mut self) {
        let t = &mut self.terminal;
        let Some(find) = &mut t.find else { return };
        let end = t
            .tabs
            .iter()
            .find(|tab| tab.id == find.tab)
            .map(|tab| tab.history.1 + tab.rows.len() as u64);
        let FindOptions {
            case_sensitive,
            whole_word,
            regex,
        } = find.options;
        let query = find.field.text();
        let compiled =
            (!query.is_empty()).then(|| TextSearch::new(query, case_sensitive, whole_word, regex));
        find.invalid = matches!(compiled, Some(Err(_)));
        find.matcher = compiled.and_then(Result::ok);
        find.matches.clear();
        find.current = None;
        find.rescan = None;
        find.stale = false;
        find.scan = end.filter(|_| find.matcher.is_some());
        self.reveal_terminal_match();
    }

    /// 标签 `id` 有新输出时调用：查找条打开时安排重新查找（间隔见 [`FIND_REFRESH`]）。
    pub(super) fn terminal_find_output(&mut self, id: u16) {
        let find = self.terminal.find.as_mut().filter(|find| find.tab == id);
        if let Some(find) = find.filter(|find| find.matcher.is_some()) {
            find.stale = true;
        }
    }

    /// 间隔已到时因新输出重新查找：先读取新进入回滚缓冲的行，查找完成后才替换结果。
    fn refresh_terminal_find(&mut self) {
        let t = &mut self.terminal;
        let Some(find) = &mut t.find else { return };
        let waiting = find.scan.is_some() || find.fetching;
        let early = find.refreshed.is_some_and(|at| at.elapsed() < FIND_REFRESH);
        if !find.stale || waiting || early {
            return;
        }
        find.stale = false;
        find.refreshed = Some(Instant::now());
        let Some(tab) = t.tabs.iter().find(|tab| tab.id == find.tab) else {
            return;
        };
        let (id, history, end) = (tab.id, tab.history, tab.history.1 + tab.rows.len() as u64);
        let fetched = (find.chunks.last())
            .map_or(history.0, |(start, lines)| start + lines.len() as u64)
            .max(history.0);
        if fetched < history.1 && find.loaded && t.client.is_some() {
            find.fetching = true;
            t.request(id, (fetched, history.1), Request::Find);
            return;
        }
        find.scan = Some(end);
        find.rescan = Some(Vec::new());
    }

    /// 画完一帧以后再找一个时间片，返回是否还没找完。
    pub(in crate::app) fn terminal_find_tick(&mut self) -> bool {
        self.refresh_terminal_find();
        self.continue_terminal_find(SLICE);
        self.terminal
            .find
            .as_ref()
            .is_some_and(|f| f.scan.is_some() || f.stale)
    }

    /// 往上继续找，最多用 `budget`；第一次找到匹配时选中最下面的那一处。
    pub(in crate::app) fn continue_terminal_find(&mut self, budget: Duration) {
        let t = &mut self.terminal;
        let Some(find) = &mut t.find else { return };
        let Some(mut next) = find.scan else { return };
        let Some(tab) = t.tabs.iter().find(|tab| tab.id == find.tab) else {
            find.scan = None;
            return;
        };
        let Some(matcher) = &find.matcher else {
            find.scan = None;
            return;
        };
        let fetched = find
            .chunks
            .last()
            .map_or(tab.history.0, |(start, lines)| start + lines.len() as u64);
        // 第 `number` 行的文字和是否软换行到下一行。依次查询屏幕、标签缓存（读取之后才进入回滚缓冲
        // 的行）和已读取的回滚缓冲。
        let text = |number: u64| -> Option<(&str, bool)> {
            if number >= tab.history.1 {
                let row = usize::try_from(number - tab.history.1).ok()?;
                return tab
                    .rows
                    .get(row)
                    .map(|line| (logical_text(line), line.wrapped));
            }
            if number >= fetched {
                return tab
                    .line(number)
                    .map(|line| (logical_text(line), line.wrapped));
            }
            let chunk = find.chunks.partition_point(|&(start, _)| start <= number);
            let (start, lines) = find.chunks.get(chunk.checked_sub(1)?)?;
            let index = usize::try_from(number - start).ok()?;
            lines
                .get(index)
                .map(|(line, wrapped)| (line.as_str(), *wrapped))
        };
        // 从下往上找到的匹配，最后倒过来接在已有的匹配前面。
        let mut found = Vec::new();
        let started = Instant::now();
        let mut joined = String::new();
        // 同一逻辑行内各行的（行号，在 `joined` 中的字节偏移，文字）。
        let mut parts: Vec<(u64, usize, &str)> = Vec::new();
        while next > tab.history.0 {
            // `next` 的上一行是逻辑行的最后一行：向上定位到行首，拼接后查找。
            let mut first = next - 1;
            while first > tab.history.0 && text(first - 1).is_some_and(|(_, wrapped)| wrapped) {
                first -= 1;
            }
            joined.clear();
            parts.clear();
            for number in first..next {
                let line = text(number).map_or("", |(line, _)| line);
                parts.push((number, joined.len(), line));
                joined.push_str(line);
            }
            // 把字节偏移 `byte` 换算成位置，所在行是最后一个起点不超过 `limit` 的行。
            let point = |byte: usize, limit: usize| {
                let at = parts
                    .partition_point(|&(_, start, _)| start <= limit)
                    .max(1)
                    - 1;
                let (line, start, part) = parts[at];
                let col = col_at(part, byte - start);
                Point { line, col }
            };
            let hits = matcher.find_all(&joined);
            let hits = hits.into_iter().rev();
            found.extend(hits.map(|(a, b)| (point(a, a), point(b, b - 1))));
            next = first;
            if started.elapsed() >= budget {
                break;
            }
        }
        find.scan = (next > tab.history.0).then_some(next);
        found.reverse();
        // 因新输出重新查找：完成后才替换结果，当前匹配保持原位置，不滚动。
        if let Some(rescan) = &mut find.rescan {
            rescan.splice(0..0, found);
            if find.scan.is_none() {
                let keep = find.current.and_then(|i| find.matches.get(i)).copied();
                find.matches = find.rescan.take().unwrap_or_default();
                find.current = match keep {
                    Some(keep) => {
                        let at = find.matches.partition_point(|m| *m < keep);
                        let last = find.matches.len().checked_sub(1);
                        last.map(|last| at.min(last))
                    }
                    None => find.matches.len().checked_sub(1),
                };
            }
            return;
        }
        let added = found.len();
        find.matches.splice(0..0, found);
        let first = find.current.is_none() && !find.matches.is_empty();
        find.current = match find.current {
            Some(i) => Some(i + added),
            None => find.matches.len().checked_sub(1),
        };
        if first {
            self.reveal_terminal_match();
        }
    }

    /// 换到上一处（`older`，往上）或下一处匹配，到头了绕回去。
    pub(in crate::app) fn terminal_find_step(&mut self, older: bool) {
        let Some(find) = &mut self.terminal.find else {
            return;
        };
        let n = find.matches.len();
        if n == 0 {
            return;
        }
        find.current = Some(match find.current {
            Some(i) if older => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None => n - 1,
        });
        self.reveal_terminal_match();
    }

    /// 选中当前的匹配，翻到能看见它的地方。
    fn reveal_terminal_match(&mut self) {
        let t = &mut self.terminal;
        let Some(find) = &t.find else { return };
        let Some(tab) = t.tabs.iter_mut().find(|tab| tab.id == find.tab) else {
            return;
        };
        let Some(&(start, end)) = find.current.and_then(|i| find.matches.get(i)) else {
            tab.selection = None;
            self.request_redraw();
            return;
        };
        tab.selection = Some(Selection {
            anchor: start,
            head: end,
            unit: Unit::Char,
        });
        tab.reveal(start.line);
        self.fetch_terminal_history();
        self.clear_find_bar(start, end);
        self.request_redraw();
    }

    /// 匹配 `start..end` 被查找条遮挡时向上滚动回滚缓冲，使所在行显示在查找条下方。
    ///
    /// 历史行不足时滚动到顶部为止。
    fn clear_find_bar(&mut self, start: Point, end: Point) {
        let Some(bar) = self.terminal_find_bar() else {
            return;
        };
        let Some(display) = self.terminal.tab().map(|tab| tab.display_of(start.line)) else {
            return;
        };
        let (first, origin) = self.terminal_origin();
        let lh = self.fonts.terminal_line_height;
        let y = origin + display.saturating_sub(first) as f32 * lh;
        let right = self.terminal_grid().x + end.col as f32 * self.metrics.terminal_cell;
        if display >= first && y < bar.bottom() && right > bar.x {
            self.scroll_terminal(((bar.bottom() - y) / lh).ceil() as isize);
        }
    }

    /// 焦点在终端的查找条里时的按键。
    pub(in crate::app) fn on_key_terminal_find(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_terminal_find(),
            Key::Named(NamedKey::Enter) => {
                let older = !self.pointer.modifiers.shift_key();
                self.terminal_find_step(older);
            }
            _ => self.on_key_field(event, base, cmd),
        }
    }
}
