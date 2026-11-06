//! 辅助进程 `winc-pty`：启动、重启和停止，同步面板尺寸和配色，收它发来的帧和回滚缓冲。

use super::*;

impl App {
    /// 启动辅助进程（已经在跑时什么也不做）。
    ///
    /// 终端标签在握手以后、知道面板尺寸时再建
    /// （[`App::sync_terminal`]）。
    pub(super) fn start_terminal(&mut self) {
        if self.terminal.client.is_some() {
            return;
        }
        let Some(path) = helper_path() else {
            self.terminal.error = Some(winc_i18n::t!("terminal-helper-missing").into());
            return;
        };
        let proxy = self.proxy.clone();
        let wake = move || {
            if let Some(proxy) = &proxy {
                let _ = proxy.send_event(UserEvent::Terminal);
            }
        };
        match Client::start(&path, wake) {
            Ok(client) => {
                client.send(&ToHelper::Theme(theme_colors(self.palette)));
                self.terminal.client = Some(client);
                self.terminal.error = None;
            }
            Err(error) => {
                let error = error.to_string();
                self.terminal.error =
                    Some(winc_i18n::t!("terminal-helper-start-failed", error = error));
            }
        }
    }

    /// 把等着发出去的新尺寸发出去（上一个尺寸的帧回来了，或者等待时间已到）；返回下一次
    /// 需要唤醒的时刻。
    pub(crate) fn terminal_resize_tick(&mut self, now: Instant) -> Option<Instant> {
        let due = self.terminal.resize_due?;
        if now < due {
            return Some(due);
        }
        // 先清掉：面板收起了、辅助进程退出了时 `sync_terminal` 什么也不做就返回，过了点的时刻
        // 如果保留，每次都会返回它，使事件循环一直空转；需要继续等待时 `sync_terminal` 会重新
        // 记上。
        self.terminal.resize_due = None;
        self.sync_terminal();
        self.terminal.resize_due
    }

    /// 光标在空行行首藏够了（见 [`TermTab::settle`]）：画出来；返回下一次要醒来的时刻。
    pub(crate) fn terminal_settle_tick(&mut self, now: Instant) -> Option<Instant> {
        let tab = self.terminal.tab_mut()?;
        let due = tab.settle? + SETTLE;
        if now < due {
            return Some(due);
        }
        tab.settle = None;
        self.request_redraw();
        None
    }

    /// 开始、停止拖分隔条：拖的时候正在看的标签先把网格上面一屏的历史行取来（`TermTab::ahead`）。
    pub(crate) fn terminal_dragging(&mut self, dragging: bool) {
        if let Some(tab) = self.terminal.tab_mut() {
            tab.ahead = dragging;
        }
        if dragging {
            self.fetch_terminal_history();
        }
    }

    /// 辅助进程意外退出了：标签和主程序这边的最后一屏留着，标签栏上写「终端组件已退出」，按 ↩
    /// 或点「重新启动」重来（[`App::restart_terminal_tab`]）。
    pub(in crate::app) fn terminal_helper_gone(&mut self) {
        let t = &mut self.terminal;
        t.client = None;
        t.ready = false;
        t.find = None;
        t.errors = None;
        t.link = None;
        t.requests.clear();
        t.resize_due = None;
        if self.focus == Focus::TerminalFind {
            self.focus = Focus::Terminal;
        }
        let t = &mut self.terminal;
        t.crashed = !t.tabs.is_empty();
        if !t.crashed {
            t.error = Some(winc_i18n::t!("terminal-helper-exited").into());
        }
    }

    /// 重新开始正在看的标签需不需要先确认：伪终端断开时里面的程序可能还在运行，重新开始会结束
    /// 它们；是否正在运行不一定查得到（Windows 上没有前台进程），所以一律确认。
    pub(in crate::app) fn terminal_restart_needs_confirm(&self) -> bool {
        let t = &self.terminal;
        self.window.is_some()
            && !t.crashed
            && self.config.terminal_confirm_close != Some(false)
            && t.tab()
                .is_some_and(|tab| tab.ended == Some(TabEnd::Disconnected))
    }

    /// 用户要重新开始（点按钮、按 ↩）：需要时先确认（[`App::terminal_restart_needs_confirm`]）。
    pub(in crate::app) fn request_terminal_restart(&mut self) {
        let Some(id) = (self.terminal_restart_needs_confirm())
            .then(|| self.terminal.tab().map(|tab| tab.id))
            .flatten()
        else {
            self.restart_terminal_tab();
            return;
        };
        let message = winc_i18n::t!("terminal-restart-confirm");
        let detail = winc_i18n::t!("terminal-restart-confirm-detail");
        let action = winc_i18n::t!("terminal-restart-confirm-action");
        self.show_confirm(message, detail, action, move |ok| {
            ok.then_some(UserEvent::TerminalRestart(id))
        });
    }

    /// 确认框上确认了重新开始编号是 `id` 的标签。
    pub(crate) fn confirmed_terminal_restart(&mut self, id: u16) {
        if let Some(index) = self.terminal.index_of(id) {
            self.terminal.active = index;
            self.restart_terminal_tab();
        }
    }

    /// 重新开始：辅助进程退出了时所有标签在各自最后的目录里重开；shell 退出了或者伪终端断开了时
    /// 只重开正在看的这一个，还在原来的位置。
    pub(in crate::app) fn restart_terminal_tab(&mut self) {
        // 正在改的名字先定下来：重开以后标签换了编号，输入框找不到它了。
        self.finish_terminal_rename(true);
        let t = &mut self.terminal;
        if t.crashed {
            t.crashed = false;
            t.want_names = t.tabs.iter().map(|tab| tab.name.clone()).collect();
            // 重开以后还看着原来看的那一个。
            t.want_active = Some(t.active);
            t.want = t.tabs.drain(..).map(|tab| tab.cwd).collect();
            t.active = 0;
            self.start_terminal();
            self.relayout();
            self.request_redraw();
            return;
        }
        let index = t.active;
        let count = t.tabs.len();
        let Some(tab) = t.tabs.get(index) else {
            return;
        };
        let (cwd, name, old) = (tab.cwd.clone(), tab.name.clone(), tab.id);
        // 先建新的再关旧的：关掉最后一个标签时辅助进程跟着退出（见 `close_terminal_tab_now`），
        // 只有这一个标签时不必为了重开换一个辅助进程。
        self.new_terminal_tab_in(cwd);
        if let Some(index) = self.terminal.tabs.iter().position(|tab| tab.id == old) {
            self.close_terminal_tab_now(index);
        }
        let t = &mut self.terminal;
        // 新标签排在最后，挪回原来的位置，名字不变（辅助进程还没准备好、标签还没建出来时名字
        // 跟着等着建的标签走，位置不挪）。
        if t.tabs.len() == count {
            let mut tab = t.tabs.pop().expect("刚建的");
            tab.name = name;
            t.tabs.insert(index, tab);
            t.active = index;
        } else if let Some(at) = t.want.len().checked_sub(1) {
            t.want_names.resize(at, String::new());
            t.want_names.push(name);
        }
    }

    /// 关掉终端模块：辅助进程连同里面的 shell 一起结束，面板收起。
    pub(crate) fn stop_terminal(&mut self) {
        self.terminal = TerminalPanel {
            height: self.terminal.height,
            ..TerminalPanel::default()
        };
        if self.focus.in_terminal() {
            self.focus = Focus::Editor;
        }
        self.relayout();
        self.request_redraw();
    }

    /// 主题变了：告诉辅助进程，程序再来问颜色时答新的。
    pub(crate) fn terminal_theme_changed(&mut self) {
        if let Some(client) = &self.terminal.client {
            client.send(&ToHelper::Theme(theme_colors(self.palette)));
        }
    }

    /// 网格放得下几列几行，以及像素尺寸（物理像素，程序查询时回答它）。
    pub(in crate::app) fn terminal_size(&self) -> Size {
        let grid = grid_rect(self.layout.terminal, self.fonts.line_height);
        let cols = (grid.w / self.cell).floor().max(2.0);
        let rows = (grid.h / self.fonts.line_height).floor().max(1.0);
        let scale = self
            .renderer
            .as_ref()
            .map_or(1.0, winc_render::Renderer::scale_factor);
        Size {
            cols: cols.min(f32::from(u16::MAX)) as u16,
            rows: rows.min(f32::from(u16::MAX)) as u16,
            width: (cols * self.cell * scale) as u16,
            height: (rows * self.fonts.line_height * scale) as u16,
        }
    }

    /// 排版之后：要新建标签时建；尺寸变了告诉每个标签；往上翻着缺历史行时取回来。
    pub(in crate::app) fn sync_terminal(&mut self) {
        if !self.terminal.open || self.layout.terminal.h <= 0.0 {
            return;
        }
        let size = self.terminal_size();
        for tab in &mut self.terminal.tabs {
            tab.shown = usize::from(size.rows);
        }
        if !self.terminal.ready {
            return;
        }
        // 要建新标签：建好以后看的是别的标签，原来那个先失去焦点（1004）。
        if self.terminal.error.is_none() && !self.terminal.want.is_empty() {
            self.blur_terminal_tab();
        }
        let cwd = self.terminal_cwd();
        let scrollback = self.config.terminal_scrollback.unwrap_or(SCROLLBACK);
        let integration = integration_dir(&self.config);
        let prompt = self.config.terminal_prompt != Some(false);
        let t = &mut self.terminal;
        let Some(client) = &t.client else {
            return;
        };
        // 每个标签同时只有一个尺寸在路上：上一个尺寸重排好的一帧回来后发下一个（中间的跳过），
        // 辅助进程和 shell 不会积压；只改正在看的标签，改宽度要重排整个回滚缓冲（一万行约 11 ms），
        // 后台标签一起排会阻塞几十毫秒，切过来时再改。
        let now = Instant::now();
        t.resize_due = None;
        let active = t.active;
        for (i, tab) in t.tabs.iter_mut().enumerate() {
            if i != active || tab.size == size || tab.ended.is_some() {
                continue;
            }
            match tab.resizing {
                Some(sent) if now < sent + RESIZE_WAIT => {
                    let due = sent + RESIZE_WAIT;
                    t.resize_due = Some(t.resize_due.map_or(due, |d| d.min(due)));
                }
                _ => {
                    tab.size = size;
                    tab.resizing = Some(now);
                    client.send(&ToHelper::Resize { tab: tab.id, size });
                }
            }
        }
        if t.error.is_none() {
            let names = std::mem::take(&mut t.want_names);
            // 等着送的输入给建好以后要看的那个标签：新建的（排在最后），或者重开时原来看的那个。
            let target = t
                .want_active
                .unwrap_or(t.tabs.len() + t.want.len().saturating_sub(1));
            for (i, want) in std::mem::take(&mut t.want).into_iter().enumerate() {
                t.next_id = t.next_id.wrapping_add(1);
                let id = t.next_id;
                client.send(&ToHelper::Spawn {
                    tab: id,
                    program: None,
                    args: Vec::new(),
                    login: true,
                    cwd: if want.is_empty() { cwd.clone() } else { want },
                    env: environment(prompt),
                    scrollback,
                    size,
                    integration: integration.clone(),
                });
                if t.tabs.len() == target
                    && let Some(bytes) = t.pending_input.take()
                {
                    client.send(&ToHelper::Input {
                        tab: id,
                        bytes,
                        rest: 0,
                    });
                }
                let mut tab = TermTab::new(id, size);
                tab.name = names.get(i).cloned().unwrap_or_default();
                t.tabs.push(tab);
                t.active = t.tabs.len() - 1;
                t.focus_sent = false;
            }
            if let Some(active) = t.want_active.take() {
                t.active = active.min(t.tabs.len().saturating_sub(1));
            }
        }
        self.fetch_terminal_history();
    }

    /// 往上翻着、缺历史行时向辅助进程要。
    pub(super) fn fetch_terminal_history(&mut self) {
        let t = &mut self.terminal;
        let Some(tab) = t.tabs.get_mut(t.active) else {
            return;
        };
        let Some(range) = tab.missing() else {
            return;
        };
        if tab.requested == Some(range) {
            return;
        }
        tab.requested = Some(range);
        let id = tab.id;
        t.request(id, range, Request::View);
    }

    /// 新终端的工作目录：打开的目录，没有时是家目录（辅助进程按空路径处理）。
    fn terminal_cwd(&self) -> Vec<u8> {
        self.editor
            .root()
            .map(|root| os_bytes(root.as_os_str()))
            .unwrap_or_default()
    }

    /// 读线程叫醒了事件循环：把收到的消息取完。
    pub(in crate::app) fn poll_terminal(&mut self) {
        loop {
            let Some(client) = &self.terminal.client else {
                return;
            };
            match client.poll() {
                Poll::Message(message) => self.terminal_message(message),
                Poll::Empty => break,
                Poll::Closed => {
                    self.terminal_helper_gone();
                    break;
                }
            }
        }
        self.request_redraw();
    }

    /// 处理辅助进程发来的一条消息。
    pub(in crate::app) fn terminal_message(&mut self, message: FromHelper) {
        let t = &mut self.terminal;
        match message {
            FromHelper::Hello { magic, version, .. } => {
                if magic == MAGIC && version == VERSION {
                    t.ready = true;
                    self.relayout();
                } else {
                    t.client = None;
                    t.error = Some(winc_i18n::t!("terminal-helper-version-mismatch").into());
                }
            }
            FromHelper::Frame { tab: id, frame } => {
                let full = frame.full;
                let mut resized = false;
                if let Some(tab) = t.by_id(id) {
                    // 等的那个尺寸的帧回来了：如果面板在这期间又变了，下一个尺寸马上发。
                    if tab.resizing.is_some()
                        && (frame.cols, frame.rows) == (tab.size.cols, tab.size.rows)
                    {
                        tab.resizing = None;
                        resized = true;
                    }
                    tab.apply(frame);
                    // 按这一帧的行号算的块先到了：现在换上。
                    if let Some(blocks) = tab.early_blocks.take() {
                        tab.set_blocks(blocks);
                    }
                    tab.credit_due = true;
                }
                if resized {
                    t.resize_due = Some(Instant::now());
                }
                if full {
                    self.terminal_renumbered(id);
                }
                self.fetch_terminal_history();
            }
            FromHelper::History {
                tab: id,
                id: request,
                start,
                lines,
                more,
            } => {
                let Some(at) = t.requests.iter().position(|r| r.0 == request) else {
                    return;
                };
                // 分几条回来的：最后一条才算这个请求结束。
                let purpose = if more {
                    t.requests[at].2
                } else {
                    t.requests.remove(at).2
                };
                match purpose {
                    Request::Find => {
                        let done = !t.waiting(id, Request::Find);
                        self.terminal_find_loaded(id, start, lines, done);
                    }
                    Request::Errors => {
                        // 分几段要的：都回来了才一起认，否则只认了第一段。
                        let Some(nav) = t.errors.as_mut().filter(|n| n.tab == id) else {
                            return;
                        };
                        nav.got.push((start, lines));
                        if !t.waiting(id, Request::Errors) {
                            let got = t.errors.as_mut().map(|n| std::mem::take(&mut n.got));
                            let got = got.unwrap_or_default();
                            let from = got.first().map_or(start, |g| g.0);
                            let lines = got.into_iter().flat_map(|g| g.1).collect();
                            self.terminal_errors_loaded(id, from, lines);
                        }
                    }
                    Request::View | Request::Copy => {
                        if let Some(tab) = t.by_id(id) {
                            tab.store_history(start, lines);
                        }
                        // 选区要的行都回来了：复制这个标签的选区。还缺的是辅助进程那边已经
                        // 不再保留的，跳过即可。
                        if purpose == Request::Copy && !t.waiting(id, Request::Copy) {
                            let text = t.by_id(id).and_then(|tab| tab.selected_text_lossy());
                            if let Some(text) = text {
                                self.set_clipboard(text, false);
                                self.flash(winc_i18n::t!("terminal-selection-copied"));
                            }
                        }
                    }
                }
            }
            FromHelper::Event { tab: id, event } => match event {
                Event::Title(title) => {
                    if let Some(tab) = t.by_id(id) {
                        tab.title = title;
                    }
                }
                Event::Clipboard(text) => self.set_clipboard(text, false),
                Event::Link(link, uri) => {
                    if let Some(tab) = t.by_id(id) {
                        tab.set_link(link, uri);
                    }
                }
                Event::Cwd(path) => {
                    if let Some(tab) = t.by_id(id) {
                        tab.cwd = path;
                    }
                }
                Event::Bell => self.terminal_alert(id, None),
                Event::Notify { title, body } => {
                    let message = match (title.is_empty(), body.is_empty()) {
                        (false, false) => {
                            winc_i18n::t!("terminal-notification", title = title, body = body)
                        }
                        (true, _) => body,
                        (false, true) => title,
                    };
                    self.terminal_alert(id, Some(message));
                }
                // 命令边界由辅助进程切成块再发（`Blocks`）。
                Event::Command(_) => {}
            },
            FromHelper::Blocks {
                tab: id,
                blocks,
                early,
            } => self.terminal_blocks(id, blocks, early),
            FromHelper::Process { tab: id, name } => {
                if let Some(tab) = t.by_id(id) {
                    tab.process = name;
                }
            }
            FromHelper::Exited { tab: id, code } => {
                if let Some(tab) = t.by_id(id) {
                    tab.ended = Some(TabEnd::Exited(code));
                }
            }
            FromHelper::Disconnected { tab: id } => {
                // 面板收着、在看别的标签时看不到标签栏上的说明：提示一声，标签上留红点。
                let seen = t.open && t.index_of(id) == Some(t.active);
                if let Some(tab) = t.by_id(id) {
                    tab.ended = Some(TabEnd::Disconnected);
                    if !seen {
                        tab.unseen = Some(true);
                    }
                }
                let message = (!seen).then(|| winc_i18n::t!("terminal-disconnected").to_string());
                self.terminal_alert(id, message);
            }
            FromHelper::Failed { tab: id, message } => {
                // 只是这个终端起不来（shell 不存在等）：说一声，其他标签和新开的终端不受影响。
                if let Some(index) = t.index_of(id) {
                    t.tabs.remove(index);
                    self.forget_terminal_tab(id);
                    let t = &mut self.terminal;
                    if t.active > index || t.active >= t.tabs.len() {
                        t.active = t.active.saturating_sub(1);
                    }
                    if t.tabs.is_empty() {
                        self.set_terminal_open(false);
                        self.focus = Focus::Editor;
                        self.relayout();
                    }
                }
                self.notify(message);
            }
        }
    }
}

/// 辅助进程的路径：环境变量 `WINC_PTY` 指定的，否则是和主程序放在一起的 `winc-pty`（Windows 上
/// 是 `winc-pty.exe`；安装包中是 `Contents/MacOS/winc-pty`，开发时是 `target/debug/winc-pty`）。
fn helper_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WINC_PTY") {
        return Some(path.into());
    }
    let exe = std::env::current_exe().ok()?;
    let path = exe
        .parent()?
        .join(format!("winc-pty{}", std::env::consts::EXE_SUFFIX));
    path.exists().then_some(path)
}

/// shell 集成脚本放在哪里（配置目录下的 `shell`）；`terminal.shellIntegration` 关了时不注入。
fn integration_dir(config: &winc_config::Config) -> Option<Vec<u8>> {
    if config.terminal_shell_integration == Some(false) {
        return None;
    }
    let config = winc_config::config_path()?;
    Some(os_bytes(config.parent()?.join("shell").as_os_str()))
}

/// 返回给终端的环境变量：winc 自己的环境原样传过去，登录 shell 会再读用户的配置补齐 `PATH`，
/// 不必先向登录 shell 查询一次。
///
/// `terminal.prompt` 关了时加上 `WINC_PROMPT=0`，集成脚本就不替换默认的提示符。
fn environment(prompt: bool) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut env: Vec<_> = std::env::vars_os()
        .filter(|(k, _)| k != "WINC_PROMPT")
        .map(|(k, v)| (os_bytes(&k), os_bytes(&v)))
        .collect();
    if !prompt {
        env.push((b"WINC_PROMPT".to_vec(), b"0".to_vec()));
    }
    env
}

/// 路径、环境变量按原样的字节传给辅助进程（它们不一定是 UTF-8）。
#[cfg(unix)]
pub(super) fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

/// 路径、环境变量按原样的字节传给辅助进程；非 Unix 平台只能先按 UTF-8 有损转换。
#[cfg(not(unix))]
pub(super) fn os_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

/// [`os_bytes`] 反过来：辅助进程报的目录换成路径。
#[cfg(unix)]
pub(super) fn bytes_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

/// [`os_bytes`] 反过来：辅助进程报的目录换成路径；非 Unix 平台按 UTF-8 有损转换。
#[cfg(not(unix))]
pub(super) fn bytes_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

// 测试用 sh 脚本冒充辅助进程。
#[cfg(all(test, unix))]
mod tests;
