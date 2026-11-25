//! 终端现场：保存工作目录、标签位置和面板状态，恢复时重新启动 shell。

use super::*;
use winc_config::{Session, TerminalEditor};
use winc_editor::DocumentId;

impl App {
    /// `slots` 只包括会保存的文件和备份；终端位置不计算临时输出和无备份的新文件。
    pub(in crate::app) fn terminal_session_with_tabs(
        &self,
        slots: &[(DocumentId, usize)],
    ) -> Option<TerminalSession> {
        let t = &self.terminal;
        // 运行配置的标签不恢复；拆分出的终端各记成一个标签。还没建出来的（面板保存时恢复的）
        // 也算，放进各自要去的那一栏。
        let mut columns: Vec<Vec<(PathBuf, String, Option<String>)>> =
            vec![Vec::new(); t.group_count()];
        let mut active = None;
        let focused = t.tab().map(|tab| tab.id);
        for (at, column) in columns.iter_mut().enumerate() {
            let shells = (t.group_tabs(at).iter().flat_map(|tab| tab.split_order()))
                .filter(|tab| tab.run.is_none());
            for tab in shells {
                if Some(tab.id) == focused {
                    active = Some((at, column.len()));
                }
                column.push((bytes_path(&tab.cwd), tab.name.clone(), tab.shell.clone()));
            }
        }
        for (i, cwd) in t.want.iter().enumerate() {
            let at = t.want_groups.get(i).copied().unwrap_or(t.group_at);
            let name = t.want_names.get(i).cloned().unwrap_or_default();
            let shell = t.want_shells.get(i).cloned().flatten();
            if let Some(column) = columns.get_mut(at) {
                column.push((bytes_path(cwd), name, shell));
            }
        }
        // 只有运行配置的那一栏不记。
        let mut placements = Vec::new();
        for (pane, id) in self.editor.panes.tree.panes().into_iter().enumerate() {
            let (tabs, current) = self.editor.pane_tabs(id);
            let mut tab = 0;
            for doc in tabs {
                let terminal = columns
                    .iter()
                    .enumerate()
                    .any(|(at, column)| !column.is_empty() && t.editor_at(at) == Some(doc));
                if terminal {
                    placements.push((
                        doc,
                        TerminalEditor {
                            pane,
                            tab,
                            active: current == Some(doc),
                        },
                    ));
                }
                if terminal || slots.iter().any(|(id, _)| *id == doc) {
                    tab += 1;
                }
            }
        }
        let active = active.map_or(0, |(at, index)| {
            columns[..at].iter().map(Vec::len).sum::<usize>() + index
        });
        let groups: Vec<TerminalGroup> = columns
            .iter()
            .enumerate()
            .filter(|(_, column)| !column.is_empty())
            .map(|(at, column)| TerminalGroup {
                tabs: column.len(),
                share: t.groups.get(at).map_or(1.0, |group| group.share),
                editor: t.editor_at(at).and_then(|id| {
                    placements
                        .iter()
                        .find(|(doc, _)| *doc == id)
                        .map(|(_, place)| *place)
                }),
            })
            .collect();
        let all: Vec<_> = columns.into_iter().flatten().collect();
        (!all.is_empty()).then(|| TerminalSession {
            active: t.want_active.unwrap_or(active),
            tabs: all.iter().map(|(cwd, _, _)| cwd.clone()).collect(),
            names: all.iter().map(|(_, name, _)| name.clone()).collect(),
            shells: (all.iter())
                .map(|(_, _, shell)| shell.clone().unwrap_or_default())
                .collect(),
            open: t.open,
            height: t.height,
            width: t.width,
            maximized: t.maximized,
            groups: if groups.len() > 1 || groups.iter().any(|g| g.editor.is_some()) {
                groups
            } else {
                Vec::new()
            },
        })
    }

    /// 打开目录时按现场重开上次的终端标签，各在原来的目录里（程序不恢复）。
    ///
    /// 已有 shell 时不动；面板保存时等打开面板再建，编辑器中的 shell 立即恢复。
    pub(in crate::app) fn restore_terminal(&mut self, session: &TerminalSession) {
        let root = self.editor.root().map(PathBuf::from);
        let enabled = self.config.terminal_enabled.unwrap_or(false);
        if !enabled || self.terminal.tab_count() > 0 || !self.terminal.want.is_empty() {
            return;
        }
        let mut markers: Vec<_> = self.terminal.groups.iter().map(|g| g.editor).collect();
        let missing = session.groups.iter().enumerate().any(|(at, group)| {
            group.editor.is_some()
                && markers
                    .get(at)
                    .copied()
                    .flatten()
                    .and_then(|id| self.editor.document(id))
                    .is_none()
        });
        if missing {
            // 通常备份恢复已放好标签；直接恢复终端时也用同一套插槽合并，避免重复创建。
            let captured: Vec<_> = self
                .editor
                .documents
                .iter()
                .filter_map(|doc| self.session_tab(doc).map(|tab| (doc.id, tab)))
                .collect();
            let mut saved = self.current_session().cloned().unwrap_or_else(|| {
                let slots: Vec<_> = captured
                    .iter()
                    .enumerate()
                    .map(|(at, (id, _))| (*id, at))
                    .collect();
                Session {
                    tabs: captured.iter().map(|(_, tab)| tab.clone()).collect(),
                    active: self.editor.active().and_then(|doc| {
                        slots
                            .iter()
                            .find(|(id, _)| *id == doc.id)
                            .map(|(_, at)| *at)
                    }),
                    panes: self.session_panes(&slots),
                    ..Session::default()
                }
            });
            saved.terminal = Some(session.clone());
            let opened: Vec<_> = saved
                .tabs
                .iter()
                .map(|tab| {
                    captured
                        .iter()
                        .find(|(_, found)| found == tab)
                        .map(|(id, _)| *id)
                })
                .collect();
            markers = crate::app::lifecycle::backup::restore_session_panes(
                &mut self.editor,
                &saved,
                &opened,
                &markers,
            );
        }
        let t = &mut self.terminal;
        // 目录按打开的目录所在的主机换算：现场中远端终端的目录可以只有那台主机上的路径，
        // 不换算会在本机启动 shell。
        let dirs: Vec<PathBuf> = (session.tabs.iter())
            .map(|dir| match &root {
                Some(root) => helper::terminal_path(root, dir.clone()),
                None => dir.clone(),
            })
            .collect();
        t.want = dirs.iter().map(|p| os_bytes(p.as_os_str())).collect();
        t.want_names.clone_from(&session.names);
        // 远端目录的 shell 是那台主机上的路径，无法在本机检查；建标签时按那台主机上检测到的
        // shell 确认。
        let saved = session.shells.iter().zip(&dirs);
        t.want_shells = saved
            .map(|(command, dir)| {
                let usable =
                    |c: &String| winc_backend::locate(dir).is_some() || shells::still_usable(c);
                Some(command.clone()).filter(|c| !c.is_empty() && usable(c))
            })
            .collect();
        t.want_active = Some(session.active);
        // 分栏：先放好各栏，标签建出来时放进各自那一栏。
        t.groups.clear();
        t.group_at = 0;
        t.panel_at = 0;
        t.want_groups.clear();
        if session.groups.len() > 1 || session.groups.iter().any(|g| g.editor.is_some()) {
            for (at, group) in session.groups.iter().enumerate() {
                // 正在用的一栏的标签在 `tabs` 里，这里仍记该栏原来的份额和编辑器标签。
                let mut restored = groups::TermGroup::empty(group.share);
                restored.editor = markers.get(at).copied().flatten();
                t.groups.push(restored);
                t.want_groups.extend(std::iter::repeat_n(at, group.tabs));
            }
            let panel = t.panel_groups().next().unwrap_or(0);
            t.panel_at = panel;
        }
        if session.height > 0.0 {
            t.height = session.height;
        }
        if session.width > 0.0 {
            t.width = session.width;
        }
        t.maximized = session.maximized;
        let editor_sessions = t.groups.iter().any(|group| group.editor.is_some());
        self.sync_terminal_editor_visibility();
        if session.open {
            self.set_terminal_open(true);
        }
        if session.open || editor_sessions {
            self.start_terminal();
        }
    }

    /// 窗口建好时按现场恢复终端：启动时恢复的目录在窗口创建之前打开，当时还不能启动辅助进程。
    ///
    /// 面板直接到位，没有打开动画；测量启动时间时不恢复。
    pub(in crate::app) fn restore_terminal_for_window(&mut self) {
        if self.frame.bench.silent() {
            return;
        }
        let session = self.current_session();
        if let Some(terminal) = session.and_then(|session| session.terminal.clone()) {
            self.restore_terminal(&terminal);
            self.terminal.motion = None;
        }
    }
}

#[cfg(test)]
mod tests;
