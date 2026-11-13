//! 在差异视图中编辑：按键、输入的文字和编辑命令转给源文档。

use super::*;

impl App {
    /// 光标在差异右侧时把按键交给右侧的文档，返回是否已处理。
    pub(in crate::app) fn diff_edit_key(&mut self, event: &KeyEvent, cmd: bool) -> bool {
        if event.logical_key == Key::Named(NamedKey::Escape) {
            let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut()) else {
                return false;
            };
            if view.read_selection.take().is_some() {
                self.request_redraw();
                return true;
            }
            if view.editing {
                view.editing = false;
                self.request_redraw();
                return true;
            }
            return false;
        }
        if edits_text(&event.logical_key) {
            self.edit_at_read_caret();
        }
        self.in_diff_source(|app| app.on_key_editor(event, cmd))
            .is_some()
    }

    /// 光标在差异右侧时把输入法提交的文字交给右侧的文档，返回是否已处理。
    pub(in crate::app) fn diff_edit_text(&mut self, text: &str) -> bool {
        self.edit_at_read_caret();
        let text = text.to_owned();
        self.in_diff_source(|app| app.run(winc_editor::Command::Insert(text)))
            .is_some()
    }

    /// 光标在差异右侧时，把撤销、剪切、粘贴、保存、行操作等编辑命令交给右侧文档。
    ///
    /// 参与转发的命令由 [`Action::edits_in_diff`] 判定；返回 `None` 表示该命令不由右侧处理。
    pub(in crate::app) fn diff_edit_action(&mut self, action: Action) -> Option<bool> {
        use Action::*;
        // 只读一侧有选区时：复制选中的内容，全选则选中该侧的全部内容。
        if let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut())
            && let Some(selection) = view.read_selection
        {
            match action {
                Copy => {
                    if let Some(text) = view.read_selected() {
                        self.set_clipboard(text, false);
                    }
                    return Some(true);
                }
                SelectAll => {
                    let side = selection.side;
                    let range = (0, view.side(side).text.len_chars());
                    view.read_selection = Some(ReadSelection { side, range });
                    self.request_redraw();
                    return Some(true);
                }
                _ => {}
            }
        }
        if !action.edits_in_diff() {
            return None;
        }
        if matches!(action, Cut | Paste) {
            self.edit_at_read_caret();
        }
        self.in_diff_source(|app| app.run_action(action))
    }

    /// 在只读光标处开始编辑：打开右侧文件，并把光标放到文档中的相同位置。
    ///
    /// 在输入文字、编辑按键和编辑命令之前调用：[`App::place_diff_caret`] 不打开文件，
    /// 用户开始编辑时才打开。光标不在可编辑的右侧或文件无法打开时不做任何操作。
    pub(in crate::app) fn edit_at_read_caret(&mut self) {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return;
        };
        let (Some((RIGHT, pos)), Some(path), false) =
            (view.read_caret(), view.new.path.clone(), view.editing)
        else {
            return;
        };
        let text = &view.new.text;
        let line = text.char_to_line(pos);
        let col = pos - text.line_to_char(line);
        let Some(source) = self.diff_source(&path, true) else {
            return;
        };
        let doc = &mut self.editor.documents[source];
        let at = doc.text().line_col_to_pos(line, col);
        doc.collapse_selections();
        doc.selection = Selection::point(at);
        doc.goal_cell = None;
        if let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut()) {
            view.editing = true;
            view.read_selection = None;
        }
    }
}
