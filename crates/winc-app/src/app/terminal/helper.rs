//! 辅助进程 `winc-pty`：启动、重启和停止，同步面板尺寸和配色，收它发来的帧和回滚缓冲。

use super::*;
use crate::app::NoticeKind;

impl App {
    /// 竖向终端列表的区域，包含新建入口和终端条目。
    pub(crate) fn terminal_tab_list(&self) -> Option<Rect> {
        self.terminal_tab_list_in(self.layout.terminal)
    }

    /// 面板在 `card` 时竖向终端列表的区域。
    fn terminal_tab_list_in(&self, card: Rect) -> Option<Rect> {
        let shown = self
            .terminal
            .strip
            .position
            .share(self.config.terminal_vertical_tabs == Some(true));
        if shown <= 0.0 {
            return None;
        }
        let top = card.y + header_height(card);
        // 约容纳 8 个汉字和关闭按钮，窄面板最多使用三分之一的宽度。
        let width = (card.w / 3.0).min(144.0).floor();
        (width > 0.0 && card.bottom() > top).then(|| {
            Rect::new(
                card.right() - width * shown,
                top,
                width,
                card.bottom() - top,
            )
        })
    }

    /// 竖向列表中的终端条目区域，顶部留给新建入口。
    pub(crate) fn terminal_tab_rows(&self) -> Option<Rect> {
        let list = self.terminal_tab_list()?;
        let top =
            (list.y + crate::theme::TREE_ROW + strip::TAB_GAP + strip::LIST_GAP).min(list.bottom());
        Some(Rect::new(list.x, top, list.w, list.bottom() - top))
    }

    /// 终端内容使用的面板范围，含顶部工具栏的高度，扣除竖向标签列表。
    pub(crate) fn terminal_content_card(&self) -> Rect {
        self.terminal_content_card_in(self.layout.terminal)
    }

    /// 面板在 `card` 时终端内容使用的范围（见 [`Self::terminal_content_card`]）。
    pub(crate) fn terminal_content_card_in(&self, card: Rect) -> Rect {
        let card = self.terminal_whole_card_in(card);
        let Some(tab) = self.terminal.tab().filter(|t| t.pane_count() > 1) else {
            return card;
        };
        let cards = super::split::pane_cards(card, tab.pane_count(), self.terminal_right());
        cards.get(tab.pane_at()).copied().unwrap_or(card)
    }

    /// 面板中除去竖排标签列表的部分，拆分前的整块。
    pub(crate) fn terminal_whole_card(&self) -> Rect {
        self.terminal_whole_card_in(self.layout.terminal)
    }

    fn terminal_whole_card_in(&self, card: Rect) -> Rect {
        Rect {
            w: (self.terminal_tab_list_in(card)).map_or(card.w, |list| list.x - card.x),
            ..card
        }
    }

    /// 面板里放网格的区域。
    ///
    /// 网格上面还有内容时贴着面板下边（[`grid_rect`]）；没有内容时和备用屏贴着标签栏，零头留在
    /// 下面，此时增减行数不会有历史行进出网格，拖动分隔条时内容也不会跳。网格装满后切换到贴着下边
    /// 时，原来的第一行成为零头处的半行，位置不变。
    pub(crate) fn terminal_grid(&self) -> Rect {
        let card = self.terminal_content_card();
        let grid = grid_rect(card, self.fonts.terminal_line_height);
        let below = |tab: &TermTab| tab.view_top() > 0 && !tab.modes.has(Modes::ALT_SCREEN);
        if self.terminal.tab().is_some_and(below) {
            return grid;
        }
        Rect {
            y: card.y + header_height(card),
            ..grid
        }
    }

    /// 显示终端内容的区域：从标签栏下沿到网格下沿，左右与网格相同。
    pub(crate) fn terminal_view(&self) -> Rect {
        let grid = self.terminal_grid();
        let top = self.layout.terminal.y + header_height(self.layout.terminal);
        Rect::new(grid.x, top, grid.w, grid.bottom() - top)
    }

    /// 视口最上面露出的显示行和它上沿的窗口坐标（对齐物理像素，文字不因滚动变模糊）。
    pub(crate) fn terminal_origin(&self) -> (u64, f32) {
        let top = self.layout.terminal.y + header_height(self.layout.terminal);
        let Some((first, cut)) = self.terminal.tab().map(TermTab::viewport) else {
            return (0, top);
        };
        let scale = (self.frame.renderer.as_ref()).map_or(1.0, winc_render::Renderer::scale_factor);
        let y = top - cut * self.fonts.terminal_line_height;
        (first, (y * scale).round() / scale)
    }

    /// 吸顶行露出的区域：紧贴标签栏，被下一条命令的提示符顶上去时变矮。
    pub(crate) fn terminal_sticky_band(&self) -> Rect {
        let card = self.terminal_content_card();
        let lh = self.fonts.terminal_line_height;
        let push = self.terminal.tab().map_or(0.0, TermTab::sticky_push);
        Rect::new(
            card.x,
            card.y + header_height(card),
            card.w,
            lh * (1.0 - push),
        )
    }

    /// 启动辅助进程（已经在跑时什么也不做）。
    ///
    /// 终端标签在握手以后、知道面板尺寸时再建
    /// （[`App::sync_terminal`]）。
    pub(super) fn start_terminal(&mut self) {
        self.detect_terminal_shells();
        if self.terminal.client.is_some() {
            return;
        }
        #[cfg(windows)]
        if !crate::windows::has_conpty() {
            self.terminal.error = Some(winc_i18n::t!("terminal-needs-conpty").into());
            return;
        }
        let Some(path) = helper_path() else {
            eprintln!("winc: 未找到 winc-pty（开发环境下请先运行 cargo build -p winc-pty）");
            self.terminal.error = Some(winc_i18n::t!("terminal-helper-missing").into());
            return;
        };
        let proxy = self.link.proxy.clone();
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
        self.window.handle.is_some()
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
            // 重开以后还看着原来看的那一个。运行配置的标签不重开，程序由用户决定是否再次运行。
            let before = t.tabs.iter().take(t.active).filter(|tab| tab.run.is_some());
            t.want_active = Some(t.active - before.count());
            t.tabs.retain(|tab| tab.run.is_none());
            t.want_names = t.tabs.iter().map(|tab| tab.name.clone()).collect();
            t.want_shells = t.tabs.iter().map(|tab| tab.shell.clone()).collect();
            t.want_into.clear();
            t.want = t.tabs.drain(..).map(|tab| tab.cwd).collect();
            if t.want.is_empty() {
                t.want.push(Vec::new());
            }
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
        if let Some(config) = tab.run.clone() {
            self.start_run(config);
            return;
        }
        let (cwd, name, old, shell) =
            (tab.cwd.clone(), tab.name.clone(), tab.id, tab.shell.clone());
        // 先建新的再关旧的：关掉最后一个标签时辅助进程跟着退出（见 `close_terminal_tab_now`），
        // 只有这一个标签时不必为了重开换一个辅助进程。
        self.new_terminal_tab_with(cwd, shell);
        self.close_terminal_now(old);
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
    ///
    /// 保留面板高度和检测到的 shell；后者与面板无关，设置中的「默认 Shell」菜单仍需列出。
    pub(crate) fn stop_terminal(&mut self) {
        self.terminal = TerminalPanel {
            height: self.terminal.height,
            shells: std::mem::take(&mut self.terminal.shells),
            shells_detected: self.terminal.shells_detected,
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
        let grid = grid_rect(
            self.terminal_content_card(),
            self.fonts.terminal_line_height,
        );
        let cols = (grid.w / self.metrics.terminal_cell).floor().max(2.0);
        let rows = (grid.h / self.fonts.terminal_line_height).floor().max(1.0);
        let scale = self
            .frame
            .renderer
            .as_ref()
            .map_or(1.0, winc_render::Renderer::scale_factor);
        Size {
            cols: cols.min(f32::from(u16::MAX)) as u16,
            rows: rows.min(f32::from(u16::MAX)) as u16,
            width: (cols * self.metrics.terminal_cell * scale) as u16,
            height: (rows * self.fonts.terminal_line_height * scale) as u16,
        }
    }

    /// 排版之后：要新建标签时建；尺寸变了告诉每个标签；往上翻着缺历史行时取回来。
    pub(in crate::app) fn sync_terminal(&mut self) {
        if !self.terminal.open || self.layout.terminal.h <= 0.0 {
            return;
        }
        let size = self.terminal_size();
        let slack = grid_slack(self.layout.terminal, self.fonts.terminal_line_height);
        for tab in &mut self.terminal.tabs {
            tab.shown = usize::from(size.rows);
            tab.slack = slack;
        }
        if !self.terminal.ready {
            return;
        }
        // 要建新标签：建好以后看的是别的标签，原来那个先失去焦点（1004）。
        let waiting = !self.terminal.want.is_empty() || !self.terminal.run_queue.is_empty();
        if self.terminal.error.is_none() && waiting {
            self.blur_terminal_tab();
        }
        // 只有新建标签时才用到；每次重绘都会运行到这里，不必每次都读取配置和拼接路径。
        // 待建标签的目录中有尚未询问过的 Python 环境时先询问，回答之后再建。
        let python = (!self.terminal.want.is_empty())
            .then(|| self.python_choices())
            .flatten();
        let spawn = python.map(|python| {
            (
                self.terminal_cwd(),
                self.config.terminal_shell.clone(),
                integration_dir(&self.config),
                python,
            )
        });
        let scrollback = self.config.terminal_scrollback.unwrap_or(SCROLLBACK);
        let prompt = self.config.terminal_prompt != Some(false);
        // 拆分过的标签：各终端按自己那一块的大小。
        let count = self.terminal.tab().map_or(1, TermTab::pane_count);
        let sizes: Vec<(u16, Size)> = (0..count)
            .filter_map(|at| {
                self.with_terminal_pane(at, |app| {
                    let id = app.terminal.tab()?.id;
                    Some((id, app.terminal_size()))
                })
            })
            .collect();
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
        let mut due = None;
        if let Some(tab) = t.tabs.get_mut(active) {
            let mut resize = |tab: &mut TermTab| {
                let size = (sizes.iter())
                    .find(|(id, _)| *id == tab.id)
                    .map_or(size, |(_, size)| *size);
                tab.shown = usize::from(size.rows);
                if tab.size == size || tab.ended.is_some() {
                    return;
                }
                match tab.resizing {
                    Some(sent) if now < sent + RESIZE_WAIT => {
                        let at = sent + RESIZE_WAIT;
                        due = Some(due.map_or(at, |d: Instant| d.min(at)));
                    }
                    _ => {
                        tab.size = size;
                        tab.resizing = Some(now);
                        client.send(&ToHelper::Resize { tab: tab.id, size });
                    }
                }
            };
            resize(tab);
            if let Some(split) = &mut tab.split {
                split.others.iter_mut().for_each(&mut resize);
            }
        }
        t.resize_due = due;
        if let Some((cwd, shell, integration, python)) = spawn.filter(|_| t.error.is_none()) {
            let names = std::mem::take(&mut t.want_names);
            let shells = std::mem::take(&mut t.want_shells);
            let intos = std::mem::take(&mut t.want_into);
            // 等着送的输入给建好以后要看的那个标签：新建的（排在最后），或者重开时原来看的那个。
            let target = t
                .want_active
                .unwrap_or(t.tabs.len() + t.want.len().saturating_sub(1));
            for (i, want) in std::mem::take(&mut t.want).into_iter().enumerate() {
                t.next_id = t.next_id.wrapping_add(1);
                let id = t.next_id;
                let dir = if want.is_empty() { cwd.clone() } else { want };
                let chosen = shells.get(i).cloned().flatten();
                // 远端的目录只接受在那台主机上检测到的 shell：本机的命令（设置里的、恢复现场
                // 时记下的）对远端不适用，不传过去，也不再记到标签上。
                let host = winc_backend::locate(&bytes_path(&dir)).map(|at| at.target);
                let chosen = match &host {
                    Some(host) => chosen.filter(|command| {
                        let known = crate::app::remote::shells(host);
                        known.iter().any(|shell| shell.command == *command)
                    }),
                    None => chosen,
                };
                // 设置中的默认 shell 是本机的命令：目录在远端主机上或 WSL 发行版中时不适用，
                // 新终端进入那台主机或发行版。
                let shell = match default_shell_applies(&dir, cfg!(windows)) {
                    true => shell.clone(),
                    false => None,
                };
                let python = python.get(i).copied().unwrap_or(false);
                let integrated = match &integration {
                    Some(_) => Integration::On { prompt, python },
                    None => Integration::Off,
                };
                let shell = chosen.as_deref().or(shell.as_deref());
                let launch = launch(shell, dir.clone(), cfg!(windows), integrated);
                client.send(&ToHelper::Spawn {
                    tab: id,
                    program: launch.program,
                    args: launch.args,
                    login: launch.login,
                    cwd: launch.cwd,
                    env: environment(prompt, python),
                    scrollback,
                    size,
                    integration: integration.clone(),
                    banner: Vec::new(),
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
                tab.slack = slack;
                tab.name = names.get(i).cloned().unwrap_or_default();
                tab.shell = chosen;
                // shell 报告当前目录之前先记下启动目录：此时重启标签或保存现场仍回到这里，
                // 远端的终端不会因目录为空而换成本机的 shell。
                tab.cwd = dir;
                // 拆分出的终端加进原来的标签，排在前台终端的后面并成为前台。
                let into = (intos.get(i).copied().flatten()).and_then(|into| t.index_of(into));
                match into {
                    Some(index) => {
                        let (mut all, at) = super::take_group(&mut t.tabs, index);
                        all.insert(at + 1, tab);
                        super::put_group(&mut t.tabs, index, all, at + 1);
                        t.active = index;
                    }
                    None => {
                        t.tabs.push(tab);
                        t.active = t.tabs.len() - 1;
                    }
                }
                t.focus_sent = false;
            }
            if let Some(active) = t.want_active.take() {
                t.active = active.min(t.tabs.len().saturating_sub(1));
            }
        }
        let mut replaced = Vec::new();
        if t.error.is_none() {
            for start in std::mem::take(&mut t.run_queue) {
                t.next_id = t.next_id.wrapping_add(1);
                let id = t.next_id;
                client.send(&ToHelper::Spawn {
                    tab: id,
                    program: start.launch.program,
                    args: start.launch.args,
                    login: start.launch.login,
                    cwd: start.launch.cwd,
                    env: start.env,
                    scrollback,
                    size,
                    integration: None,
                    banner: start.banner,
                });
                let mut tab = TermTab::new(id, size);
                tab.slack = slack;
                tab.name.clone_from(&start.config.name);
                tab.run = Some(start.config);
                t.tabs.push(tab);
                t.active = t.tabs.len() - 1;
                t.focus_sent = false;
                replaced.extend(start.replaces.map(|old| (id, old)));
            }
        }
        self.retire_run_tabs(replaced);
        self.drop_other_tab_state();
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

    /// 返回各个待建标签是否加载启动目录中的 Python 环境；有环境尚未询问过时弹出提示并返回
    /// `None`，回答之后再建。
    ///
    /// 环境里的程序排在 `PATH` 最前，会先于系统中的同名命令运行，因此每个环境只在用户同意后加载；
    /// 回答按环境的目录记录（见 [`App::answered_python_environment`]）。没有窗口（测试）时无法
    /// 询问，按同意处理。
    pub(in crate::app) fn python_choices(&mut self) -> Option<Vec<bool>> {
        let count = self.terminal.want.len();
        if self.config.terminal_shell_integration == Some(false) {
            return Some(vec![false; count]);
        }
        if self.terminal.python_asking.is_some() {
            return None;
        }
        let root = self.terminal_cwd();
        let mut choices = Vec::with_capacity(count);
        for want in &self.terminal.want {
            let dir = bytes_path(if want.is_empty() { &root } else { want });
            // 没有打开目录时终端从用户的家目录启动，那里的环境是用户自己的，不询问。
            if dir.as_os_str().is_empty() {
                choices.push(true);
                continue;
            }
            let candidates = [".venv", "venv"].map(|name| dir.join(name));
            // 远端的目录不能在界面线程上查询：按文件树上看得到的目录判断，看不到时不加载，
            // 不会在没有询问的情况下加载远端的环境。
            let venv = if winc_backend::is_remote(&dir) {
                let listed = |venv: &PathBuf| self.known_folder(venv) == Some(true);
                match candidates.into_iter().find(listed) {
                    Some(venv) => Some(venv),
                    None => {
                        choices.push(false);
                        continue;
                    }
                }
            } else {
                let found = |venv: &PathBuf| winc_backend::fs::is_file(venv.join("pyvenv.cfg"));
                candidates.into_iter().find(found)
            };
            let Some(venv) = venv else {
                choices.push(true);
                continue;
            };
            // 只有同意被长期记录（记为 1）；拒绝只在本次运行中有效，不会永久关闭这个环境的加载。
            if self.terminal.python_declined.contains(&venv) {
                choices.push(false);
            } else if self.state.trusted_repo(&venv) == Some(1) || self.window.handle.is_none() {
                choices.push(true);
            } else {
                self.ask_python_environment(venv);
                return None;
            }
        }
        Some(choices)
    }

    /// 询问是否为终端加载 Python 环境 `venv`。
    fn ask_python_environment(&mut self, venv: PathBuf) {
        let message = winc_i18n::t!("terminal-python-confirm");
        let path = venv.display().to_string();
        let detail = winc_i18n::t!("terminal-python-confirm-detail", path = path);
        use crate::app::lifecycle::alert::AlertButton;
        let buttons = vec![
            AlertButton::new(winc_i18n::t!("terminal-python-confirm-action")),
            AlertButton::cancel(winc_i18n::t!("terminal-python-confirm-decline")),
        ];
        let asked = venv.clone();
        let shown = self.show_alert(message, &detail, buttons, None, move |index, _| {
            Some(UserEvent::PythonEnvironment(asked.clone(), index == 0))
        });
        // 提示框暂时无法弹出（已有别的提示框）时不记录，下一次排版再问。
        if shown {
            self.terminal.python_asking = Some(venv);
        }
    }

    /// 提示框中回答了是否加载 Python 环境 `venv`，然后建等着的标签。
    ///
    /// 同意长期记录，同一个环境不再询问；拒绝只记到本次运行结束。
    pub(crate) fn answered_python_environment(&mut self, venv: PathBuf, load: bool) {
        if load {
            self.terminal
                .python_declined
                .retain(|declined| *declined != venv);
            self.update_state(|state| state.trust_repo(&venv, 1));
        } else if !self.terminal.python_declined.contains(&venv) {
            self.terminal.python_declined.push(venv);
        }
        self.terminal.python_asking = None;
        self.relayout();
        self.request_redraw();
    }

    /// 读线程叫醒了事件循环：把收到的消息取完。
    ///
    /// 只有消息改变了界面上的内容时才重绘：后台标签和收起的面板持续发送标题、进程、命令块时不重绘
    /// 窗口。提示通过 `notify`、`flash` 显示，它们自行请求重绘。
    pub(in crate::app) fn poll_terminal(&mut self) {
        let before = self.terminal.visible_state();
        let mut shown = false;
        loop {
            let Some(client) = &self.terminal.client else {
                self.request_redraw();
                return;
            };
            match client.poll() {
                Poll::Message(message) => {
                    shown |= self.terminal.message_is_shown(&message);
                    self.terminal_message(message);
                }
                Poll::Empty => break,
                Poll::Closed => {
                    self.terminal_helper_gone();
                    shown = true;
                    break;
                }
            }
        }
        if shown || self.terminal.visible_state() != before {
            self.request_redraw();
        }
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
                self.terminal_find_output(id);
                let t = &mut self.terminal;
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
                            let text = t.by_id(id).and_then(|tab| {
                                let text = tab.selected_text_lossy();
                                if std::mem::take(&mut tab.clear_after_copy) {
                                    tab.selection = None;
                                }
                                text
                            });
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
                // 只接受正在看的标签写剪贴板：看不见的标签里的输出不能在用户不知情时改写它。
                Event::Clipboard(text) => {
                    if t.open && t.index_of(id) == Some(t.active) {
                        self.set_clipboard(text, false);
                    }
                }
                Event::Link(link, uri) => {
                    if let Some(tab) = t.by_id(id) {
                        tab.set_link(link, uri);
                    }
                }
                Event::Cwd(path) => {
                    let path = local_cwd(self.editor.root(), path);
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
                let mut again = None;
                self.run_tab_exited(id, code);
                let t = &mut self.terminal;
                if let Some(tab) = t.by_id(id) {
                    tab.ended = Some(TabEnd::Exited(code));
                    if std::mem::take(&mut tab.rerun) {
                        again.clone_from(&tab.run);
                    }
                }
                if let Some(config) = again {
                    self.start_run(config);
                    return;
                }
                // shell 正常退出（`exit`、⌃D）时关掉标签，不留一个等回车的空标签；异常退出的
                // 和运行配置的标签留着，用户要看退出码和输出。
                // 拆分出的终端也一样，只关它自己。
                let shell = (self.terminal.tabs.iter().flat_map(TermTab::panes))
                    .any(|tab| tab.id == id && tab.run.is_none());
                if code == 0 && shell {
                    self.close_terminal_now(id);
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
                if let Some((index, gone)) = t.remove_terminal(id) {
                    self.forget_terminal_tab(id);
                    let t = &mut self.terminal;
                    if gone && (t.active > index || t.active >= t.tabs.len()) {
                        t.active = t.active.saturating_sub(1);
                    }
                    if t.tabs.is_empty() {
                        self.set_terminal_open(false);
                        self.focus = Focus::Editor;
                        self.relayout();
                    }
                }
                self.notify(NoticeKind::Error, message);
            }
        }
    }
}

/// 辅助进程的路径：环境变量 `WINC_PTY` 指定的，否则是和主程序放在一起的 `winc-pty`（Windows 上
/// 是 `winc-pty.exe`；安装包中是 `Contents/MacOS/winc-pty`，开发时是 `target/debug/winc-pty`）。
#[allow(
    clippy::disallowed_methods,
    reason = "只访问本机：辅助进程与主程序在同一目录"
)]
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

/// 设置中的默认 shell 是否用于在目录 `dir` 新建的终端：目录在远端主机上，或在 Windows 上位于
/// WSL 发行版中时不用。菜单中明确选择的 shell 不受此限制。
pub(super) fn default_shell_applies(dir: &[u8], windows: bool) -> bool {
    let dir = bytes_path(dir);
    let in_distro = windows && winc_editor::wsl::locate(&dir).is_some();
    !winc_backend::is_remote(&dir) && !in_distro
}

/// 把 shell 报告的工作目录 `path` 转换为编辑器使用的路径（见 [`terminal_path`]）。重启标签和
/// 恢复现场都使用该目录。
fn local_cwd(root: Option<&std::path::Path>, path: Vec<u8>) -> Vec<u8> {
    match root {
        Some(root) => os_bytes(terminal_path(root, bytes_path(&path)).as_os_str()),
        None => path,
    }
}

/// 把终端中的路径 `path` 转换为编辑器使用的路径：打开的目录 `root` 在远端主机上时，`/` 开头的
/// 路径是那台主机上的路径；位于 WSL 发行版中时属于同一发行版。其余路径原样返回。
///
/// 已经是远端主机上的路径时原样返回：标签记下的目录、恢复的现场都是换算过的，Unix 上这种路径
/// 同样以 `/` 开头，再换算一次会指向错误的位置。
pub(in crate::app) fn terminal_path(root: &std::path::Path, path: PathBuf) -> PathBuf {
    if winc_backend::is_remote(&path) {
        return path;
    }
    let bytes = os_bytes(path.as_os_str());
    match winc_backend::locate(root) {
        Some(at) if bytes.first() == Some(&b'/') => winc_backend::virtual_path(&at.target, &bytes),
        Some(_) => path,
        None => winc_editor::wsl::from_terminal(root, path),
    }
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
/// 远端主机上的 shell 是否注入集成脚本；本机的 shell 由辅助进程按脚本目录决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Integration {
    /// 不注入：设置里关了，或要运行的是一条命令。
    Off,
    /// 注入；`prompt` 为假时不换上 winc 的提示符（`terminal.prompt`），`python` 为假时不加载
    /// 启动目录中的 Python 环境。
    On { prompt: bool, python: bool },
}

/// 新终端的启动方式：程序、参数和工作目录。
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Launch {
    /// 要运行的程序；`None` 表示用户的默认 shell。
    pub(super) program: Option<Vec<u8>>,
    pub(super) args: Vec<Vec<u8>>,
    pub(super) login: bool,
    pub(super) cwd: Vec<u8>,
}

/// 返回在目录 `cwd` 新建终端时的启动方式。
///
/// `cwd` 在远端主机上时运行 `ssh` 登录那台主机并进入该目录，见 [`remote_launch`]。
///
/// 设置了 `shell` 时运行该命令（分词见 [`super::shells::command_words`]）。在 Windows（`windows`）
/// 上且 `cwd` 位于 WSL 发行版中时，进入对应的发行版和目录：目录由 `wsl.exe` 的参数指定，进程本身
/// 从家目录启动。其余情况运行默认 shell。
pub(super) fn launch(
    shell: Option<&str>,
    cwd: Vec<u8>,
    windows: bool,
    integration: Integration,
) -> Launch {
    if let Some(at) = winc_backend::locate(&bytes_path(&cwd)) {
        let ready = crate::app::remote::shell_ready(&at.target);
        // 只接受那台主机上的绝对路径（由调用方按检测到的 shell 确认）；本机的命令不适用。
        let shell = shell
            .and_then(super::shells::program_path)
            .filter(|path| path.starts_with('/'));
        return remote_launch(&at, integration, ready, shell.as_deref());
    }
    // Windows 上 `/` 开头的目录来自 WSL 中的 shell，不是 Windows 程序可用的工作目录。
    let cwd = if windows && cwd.first() == Some(&b'/') {
        Vec::new()
    } else {
        cwd
    };
    let mut words = shell
        .map(super::shells::command_words)
        .unwrap_or_default()
        .into_iter();
    if let Some(program) = words.next() {
        return Launch {
            program: Some(program.into_bytes()),
            args: words.map(String::into_bytes).collect(),
            login: !windows,
            cwd,
        };
    }
    let dir = String::from_utf8_lossy(&cwd);
    let place = windows
        .then(|| winc_editor::wsl::locate(std::path::Path::new(dir.as_ref())))
        .flatten();
    let Some(place) = place else {
        return Launch {
            program: None,
            args: Vec::new(),
            login: true,
            cwd,
        };
    };
    let args = ["-d", place.distro.as_str(), "--cd", place.path.as_str()];
    Launch {
        program: Some(b"wsl.exe".to_vec()),
        args: args.iter().map(|arg| arg.as_bytes().to_vec()).collect(),
        login: false,
        cwd: Vec::new(),
    }
}

/// 返回在远端主机上的目录 `at` 新建终端时的启动方式：在本机的终端里运行 `ssh`，登录后进入该
/// 目录并换成那台主机上用户的登录 shell；启用了 shell 集成时一并注入（命令块、当前目录）。
///
/// 编辑器的连接可以复用时（见 `winc_backend::remote::shared`），终端里的 `ssh` 经它开会话，
/// 不再登录；否则是另一条连接，需要口令时在终端里输入，为该主机指定了密钥文件时也用它。`-t` 分配
/// 终端；`RemoteCommand=none` 使配置中为该主机设置的登录命令不与这里的命令冲突。远端的命令
/// 先经用户的登录 shell 解释，写法只用各种 shell 共有的部分；目录包在单引号内。设置中的
/// `shell` 是本机的命令，对远端的目录不适用。
fn remote_launch(
    at: &winc_backend::Location,
    integration: Integration,
    ready: bool,
    shell: Option<&str>,
) -> Launch {
    // 集成脚本已写到那台主机上（`ready`）时由其中的启动脚本启动 shell（见
    // `winc_pty::scripts`），否则直接启动登录 shell。`shell` 是那台主机上的路径，由调用方
    // 确认过在那台主机上存在。
    let start = match (integration, shell) {
        (Integration::On { prompt, python }, _) if ready => {
            winc_pty::scripts::remote_command(prompt, python, shell)
        }
        (_, Some(shell)) => format!("exec {} -l", winc_pty::scripts::single_quoted(shell)),
        (_, None) => "exec $SHELL -l".to_owned(),
    };
    let dir = String::from_utf8_lossy(&at.path).replace('\'', r"'\''");
    let mut args: Vec<String> = ["-t", "-o", "RemoteCommand=none"].map(str::to_owned).into();
    // 编辑器的连接可以复用时经它开会话，不再握手和认证；用不上时 `ssh` 自行单独登录。
    if let Some(options) = winc_backend::remote::shared(&at.target) {
        args.extend(options);
    }
    // 与编辑器的连接用同一个密钥文件。
    if let Some(key) = winc_backend::remote::identity(&at.target) {
        args.extend(["-i".to_owned(), key]);
    }
    if let Some(port) = at.target.port() {
        args.extend(["-p".to_owned(), port.to_string()]);
    }
    args.extend([
        at.target.host().to_owned(),
        "--".to_owned(),
        format!("cd '{dir}' && {start}"),
    ]);
    Launch {
        program: Some(b"ssh".to_vec()),
        args: args.into_iter().map(String::into_bytes).collect(),
        login: false,
        cwd: Vec::new(),
    }
}

/// 返回给终端的环境变量；`prompt`、`python` 为假时分别加上 `WINC_PROMPT=0`、`WINC_PYTHON=0`，
/// 集成脚本据此不替换默认提示符、不加载启动目录中的 Python 环境。
pub(super) fn environment(prompt: bool, python: bool) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut env: Vec<_> = std::env::vars_os()
        .filter(|(k, _)| k != "WINC_PROMPT" && k != "WINC_PYTHON")
        .map(|(k, v)| (os_bytes(&k), os_bytes(&v)))
        .collect();
    if !prompt {
        env.push((b"WINC_PROMPT".to_vec(), b"0".to_vec()));
    }
    if !python {
        env.push((b"WINC_PYTHON".to_vec(), b"0".to_vec()));
    }
    // 从图形界面启动时没有区域设置：按系统的首选语言补上 `LANG`。系统没有对应的区域数据时
    // 不补，由辅助进程只声明字符编码。
    #[cfg(target_os = "macos")]
    if !env
        .iter()
        .any(|(k, _)| matches!(&k[..], b"LANG" | b"LC_ALL" | b"LC_CTYPE"))
        && let Some(name) = winc_i18n::Locale::system_posix()
        && winc_backend::fs::is_dir(
            std::path::Path::new("/usr/share/locale").join(format!("{name}.UTF-8")),
        )
    {
        env.push((b"LANG".to_vec(), format!("{name}.UTF-8").into_bytes()));
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
