//! 键位方案和改过的快捷键：切换方案、导入，在「快捷键」页中修改、添加、移除一个命令的快捷键
//! 和恢复默认。
//!
//! 改动写入配置文件的 `keyboard` 一节，随即重新安装键位表（[`keymap::install_from`]）。

use super::*;
use crate::keymap::{Action, Custom, Keys, Preset};
use crate::layout::KeyRows;

/// 「快捷键」页中正在修改快捷键的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyEdit {
    /// 修改的命令。
    pub action: Action,
    /// 保留的已有快捷键；点击键帽上的 × 去掉。
    pub keys: Vec<Keys>,
    /// 新按下的快捷键，保存时加在保留的键之后；还没有按时为 `None`。
    pub recorded: Option<Keys>,
}

impl App {
    /// 键位表换过之后：重排「快捷键」页并重绘，菜单中的快捷键随之更新。
    pub(crate) fn keymap_changed(&mut self) {
        self.relayout();
        self.request_redraw();
    }

    /// 返回「快捷键」页顶部和列表中窗口坐标 (`x`, `y`) 处的部件；`body` 是设置标题下方的区域。
    pub(super) fn keys_part_at(&self, body: Rect, x: f32, y: f32) -> Option<SettingsPart> {
        let header = KeysLayout::header(body);
        if header.record.contains(x, y) {
            return Some(SettingsPart::Record);
        }
        if header.search.contains(x, y) {
            return Some(SettingsPart::KeySearch);
        }
        if header.value.contains(x, y) {
            return Some(SettingsPart::Keymap);
        }
        let keys = self.keys_layout();
        if !keys.list.contains(x, y) {
            return None;
        }
        let drawn = (self.hits.keys_parts.iter()).find(|(rect, _)| rect.contains(x, y));
        if let Some(&(_, part)) = drawn {
            return Some(part);
        }
        (keys.rows())
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, row)| SettingsPart::KeyRow(row.action))
    }

    /// 处理「快捷键」页列表中的点击 `part`：点击一行或「编辑」展开该行开始修改，展开后点击 ×
    /// 去掉一个已有的键，「保存」「取消」结束修改。
    pub(in crate::app) fn click_keys_part(&mut self, part: SettingsPart) {
        let editing = self.settings.key_edit.as_ref().map(|edit| edit.action);
        match part {
            SettingsPart::KeyRow(action) | SettingsPart::KeyEdit(action)
                if editing != Some(action) =>
            {
                self.start_key_edit(action, true);
            }
            SettingsPart::KeyReset(action) => self.reset_shortcuts(action),
            SettingsPart::KeyRemove(index) => {
                if let Some(edit) = &mut self.settings.key_edit
                    && index < edit.keys.len()
                {
                    edit.keys.remove(index);
                }
            }
            SettingsPart::KeySave => self.commit_key_edit(),
            SettingsPart::KeyCancel => self.cancel_key_edit(),
            _ => {}
        }
        self.request_redraw();
    }

    /// 点击 `part` 时是否保留正在进行的修改：点在展开的那一行和其中的按钮上。
    pub(in crate::app) fn keeps_key_edit(&self, part: Option<SettingsPart>) -> bool {
        let editing = self.settings.key_edit.as_ref().map(|edit| edit.action);
        match part {
            Some(SettingsPart::KeyRemove(_) | SettingsPart::KeySave | SettingsPart::KeyCancel) => {
                true
            }
            Some(SettingsPart::KeyRow(action)) => editing == Some(action),
            _ => false,
        }
    }

    /// 在「键位方案」的值下方弹出菜单：各方案、从本机找到的编辑器导入和恢复全部默认。
    pub(in crate::app) fn open_keymap_menu(&mut self) {
        self.cancel_key_edit();
        self.settings.import_sources = keymap::import::find_sources();
        let header = KeysLayout::header(self.settings_layout().body);
        let at = menu_below(header.value);
        self.open_context_menu(MenuTarget::Keymap, at);
    }

    /// 改用键位方案 `preset`；改过的键保留。
    pub(in crate::app) fn set_keymap_preset(&mut self, preset: Preset) {
        if keymap::current_preset() == preset {
            return;
        }
        self.config.keymap = Some(preset.key().to_owned());
        self.save_config();
        self.reinstall_keymap();
    }

    /// 按 `change` 修改改过的快捷键，写入配置文件并重新安装键位表。
    pub(in crate::app) fn change_shortcuts(&mut self, change: impl FnOnce(&mut Custom)) {
        let (mut custom, _) = Custom::from_config(&self.config.shortcuts);
        change(&mut custom);
        self.config.shortcuts = custom.to_config(Os::CURRENT);
        self.save_config();
        self.reinstall_keymap();
    }

    /// 按配置重新安装键位表。
    fn reinstall_keymap(&mut self) {
        for problem in keymap::install_from(&self.config) {
            eprintln!("winc: {problem}");
        }
        self.keymap_changed();
    }

    /// 命令 `action` 是否有改过的快捷键。
    pub(crate) fn shortcut_customized(&self, action: Action) -> bool {
        self.config.shortcuts.contains_key(&action.id())
    }

    /// 开始修改命令 `action` 的快捷键：`keep` 为真时保留已有的键，否则从空白开始。
    ///
    /// 修改期间按键都记作新键，不执行；macOS 上菜单栏的快捷键暂停。
    pub(in crate::app) fn start_key_edit(&mut self, action: Action, keep: bool) {
        self.set_recording(false);
        let closing = (self.settings.key_edit.as_ref())
            .map(|edit| edit.action)
            .filter(|&a| a != action);
        self.slide_key_rows(closing);
        let keys = if keep {
            self.current_keys(action)
        } else {
            Vec::new()
        };
        self.settings.key_edit = Some(KeyEdit {
            action,
            keys,
            recorded: None,
        });
        self.suspend_menu_keys(true);
        self.focus = Focus::Settings;
        self.reveal_key_row(action);
        self.request_redraw();
    }

    /// 滚动「快捷键」页的列表，使命令 `action` 展开后的一行整个可见。
    fn reveal_key_row(&mut self, action: Action) {
        let open = Some(action);
        let keys = self.keys_layout_with(KeyRows {
            open,
            closing: None,
            t: 1.0,
        });
        let Some((row, _)) = keys.rows().find(|(_, row)| row.action == action) else {
            return;
        };
        let list = keys.list;
        let shift = if row.bottom() > list.bottom() {
            (row.bottom() - list.bottom()).min(row.y - list.y)
        } else {
            (row.y - list.y).min(0.0)
        };
        self.settings.scroll = (self.settings.scroll + shift).clamp(0.0, keys.max_scroll());
    }

    /// 结束修改快捷键，不保存。
    pub(in crate::app) fn cancel_key_edit(&mut self) {
        if let Some(edit) = self.settings.key_edit.take() {
            self.slide_key_rows(Some(edit.action));
            self.suspend_menu_keys(false);
            self.request_redraw();
        }
    }

    /// 开始一行展开、收起的过渡，同时收起的是命令 `closing` 的一行。
    fn slide_key_rows(&mut self, closing: Option<Action>) {
        let clock = self.clock(super::super::animation::TREE_ANIMATION);
        self.settings.key_slide = clock.map(|clock| (closing, clock));
    }

    /// 处理修改快捷键时的一次按键 `stroke`：`↩` 保存，`Esc` 放弃，其余按键记作新键。
    ///
    /// 新键最多两段（组合键）；已有两段时再按一次重新开始。
    pub(in crate::app) fn key_edit_stroke(&mut self, stroke: Stroke) {
        use keymap::{KeyCode, Mods};
        let Some(edit) = &mut self.settings.key_edit else {
            return;
        };
        let plain = stroke.mods == Mods::NONE;
        match stroke.key {
            KeyCode::Escape if plain => return self.cancel_key_edit(),
            KeyCode::Enter if plain => return self.commit_key_edit(),
            _ => {}
        }
        edit.recorded = match edit.recorded {
            Some((first, None)) => Some((first, Some(stroke))),
            _ => Some((stroke, None)),
        };
        self.request_redraw();
    }

    /// 保存正在修改的快捷键：保留的已有键加上新按下的键；没有变化时只结束修改。与新键冲突的
    /// 命令中，改过的那些也在配置中去掉该键。
    fn commit_key_edit(&mut self) {
        let conflicts = self.key_edit_conflicts();
        let Some(edit) = self.settings.key_edit.take() else {
            return;
        };
        self.slide_key_rows(Some(edit.action));
        self.suspend_menu_keys(false);
        self.request_redraw();
        let mut keys = edit.keys;
        if let Some(new) = edit.recorded
            && !keys.contains(&new)
        {
            keys.push(new);
        }
        if keys != self.current_keys(edit.action) {
            self.change_shortcuts(|custom| {
                if let Some(new) = edit.recorded {
                    for other in conflicts {
                        custom.release(other, new);
                    }
                }
                custom.set(edit.action, keys);
            });
        }
    }

    /// 返回命令 `action` 当前的全部键（去掉重复）。
    pub(crate) fn current_keys(&self, action: Action) -> Vec<Keys> {
        let mut keys: Vec<Keys> = Vec::new();
        for b in keymap::current().iter().filter(|b| b.action == action) {
            if !keys.contains(&(b.first, b.second)) {
                keys.push((b.first, b.second));
            }
        }
        keys
    }

    /// 返回新按下的快捷键与之冲突的其他命令：保存后这些命令不再使用该键。
    pub(crate) fn key_edit_conflicts(&self) -> Vec<Action> {
        let Some(KeyEdit {
            action,
            recorded: Some((first, second)),
            ..
        }) = self.settings.key_edit.as_ref().cloned()
        else {
            return Vec::new();
        };
        let table = keymap::current();
        let scopes = action_scopes(&table, action);
        let mut found = Vec::new();
        for b in table.iter() {
            let overlaps = scopes.iter().any(|&when| b.when.overlaps(when));
            let clashes = keymap::clashes((b.first, b.second), (first, second));
            if b.action != action && overlaps && clashes && !found.contains(&b.action) {
                found.push(b.action);
            }
        }
        found
    }

    /// 新按下的快捷键是否会取代文字输入：单段、不带 Ctrl、Alt、⌘ 的按键（功能键等除外），且该
    /// 命令在编辑区或输入框中生效。
    pub(crate) fn key_edit_takes_typing(&self) -> bool {
        use keymap::{KeyCode, Mods, When};
        let Some(KeyEdit {
            action,
            recorded: Some((first, None)),
            ..
        }) = self.settings.key_edit.as_ref().cloned()
        else {
            return false;
        };
        let plain = [Mods::CTRL, Mods::ALT, Mods::META]
            .into_iter()
            .all(|bit| !first.mods.has(bit));
        let typing = !matches!(
            first.key,
            KeyCode::F(_) | KeyCode::Escape | KeyCode::Insert | KeyCode::Tap
        );
        let scopes = action_scopes(&keymap::current(), action);
        let input =
            (scopes.iter()).any(|w| w.overlaps(When::EditorOrFind) || w.overlaps(When::Panel));
        plain && typing && input
    }

    /// 从「键位方案」菜单中第 `index` 处来源导入快捷键（见 [`App::import_keymap_from`]）。
    pub(in crate::app) fn import_keymap(&mut self, index: usize) {
        if let Some(source) = self.settings.import_sources.get(index).cloned() {
            self.import_keymap_from(&source);
        }
    }

    /// 从来源 `source` 导入快捷键：改用其键位方案，改过的键全部换成它的；结果用提示说明。
    pub(in crate::app) fn import_keymap_from(&mut self, source: &keymap::import::Source) {
        match keymap::import::read(source, Os::CURRENT) {
            Ok(imported) => {
                self.adopt_imported_keymap(&imported);
                let name = source.name.as_str();
                match imported.skipped {
                    0 => self.notify(NoticeKind::Done, t!("settings-keys-imported", name = name)),
                    n => self.notify(
                        NoticeKind::Warning,
                        t!("settings-keys-imported-skipped", name = name, count = n),
                    ),
                }
            }
            Err(reason) => self.notify(NoticeKind::Error, reason),
        }
    }

    /// 改用导入的键位方案和改过的键，写入配置文件并重新安装键位表。
    pub(in crate::app) fn adopt_imported_keymap(&mut self, imported: &keymap::import::Imported) {
        self.config.keymap = Some(imported.preset.key().to_owned());
        self.config.shortcuts = imported.custom.to_config(Os::CURRENT);
        self.save_config();
        self.reinstall_keymap();
    }

    /// 移除命令 `action` 的全部快捷键。
    pub(in crate::app) fn remove_shortcuts(&mut self, action: Action) {
        self.change_shortcuts(|custom| custom.set(action, Vec::new()));
    }

    /// 恢复命令 `action` 在键位方案中的快捷键。
    pub(in crate::app) fn reset_shortcuts(&mut self, action: Action) {
        self.change_shortcuts(|custom| custom.reset(action));
    }

    /// 恢复全部命令在键位方案中的快捷键。
    pub(in crate::app) fn reset_all_shortcuts(&mut self) {
        self.change_shortcuts(|custom| *custom = Custom::default());
    }

    /// 暂停（`on` 为真）或恢复 macOS 菜单栏中的快捷键，使按键送到窗口。
    fn suspend_menu_keys(&self, on: bool) {
        // 没有窗口（测试中）时没有菜单栏，也不在主线程上。
        #[cfg(target_os = "macos")]
        if self.window.handle.is_some() {
            crate::macos::suspend_menu_shortcuts(on);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = on;
    }
}

/// 返回命令 `action` 在键位表 `table` 中的生效范围；没有键时为所有区域。
fn action_scopes(table: &[keymap::Binding], action: Action) -> Vec<keymap::When> {
    let mut scopes: Vec<keymap::When> = Vec::new();
    for b in table.iter().filter(|b| b.action == action) {
        if !scopes.contains(&b.when) {
            scopes.push(b.when);
        }
    }
    if scopes.is_empty() {
        scopes.push(keymap::When::Global);
    }
    scopes
}
