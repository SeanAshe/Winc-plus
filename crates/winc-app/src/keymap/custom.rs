//! 用户改过的快捷键：按命令整组替换方案中的键，写在配置文件的 `keyboard.shortcuts` 中。
//!
//! 一个命令的键写成一个数组，空数组表示该命令不设快捷键；没写的命令沿用方案中的键。改过的键
//! 在其生效范围内取代其他命令的同一个键。

use std::collections::BTreeMap;

use winc_i18n::t;

use super::*;

/// 一条绑定的按键：第一段和组合键的第二段。
pub type Keys = (Stroke, Option<Stroke>);

/// 用户改过的快捷键，按命令在 [`ACTIONS`] 中的顺序排列。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Custom {
    entries: Vec<(Action, Vec<Keys>)>,
}

impl Custom {
    /// 读入配置文件中的 `shortcuts`（命令名 → 按键写法），返回改过的键和读入时的问题。
    ///
    /// 不认识的命令名和写法不对的键记入问题后跳过。
    pub fn from_config(map: &BTreeMap<String, Vec<String>>) -> (Custom, Vec<String>) {
        let mut custom = Custom::default();
        let mut problems = Vec::new();
        for (id, specs) in map {
            let Some(action) = Action::from_id(id) else {
                problems.push(t!("config-shortcut-unknown-command", name = id.as_str()));
                continue;
            };
            let mut keys = Vec::new();
            for spec in specs {
                match parse_spec(spec) {
                    Some(parsed) if !keys.contains(&parsed) => keys.push(parsed),
                    Some(_) => {}
                    None => problems.push(t!(
                        "config-shortcut-invalid-key",
                        name = id.as_str(),
                        key = spec.as_str()
                    )),
                }
            }
            custom.set(action, keys);
        }
        (custom, problems)
    }

    /// 返回写入配置文件的内容：命令名 → `os` 上的按键写法。
    pub fn to_config(&self, os: Os) -> BTreeMap<String, Vec<String>> {
        (self.entries.iter())
            .map(|(action, keys)| {
                let specs = keys.iter().map(|&(a, b)| binding_spec(a, b, os)).collect();
                (action.id(), specs)
            })
            .collect()
    }

    /// 是否没有改过任何键。
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 返回命令 `action` 改过的键；没改过时返回 `None`。
    #[cfg(test)]
    pub fn keys(&self, action: Action) -> Option<&[Keys]> {
        (self.entries.iter())
            .find(|(a, _)| *a == action)
            .map(|(_, keys)| keys.as_slice())
    }

    /// 把命令 `action` 的键改为 `keys`，空表示不设快捷键。
    pub fn set(&mut self, action: Action, keys: Vec<Keys>) {
        match self.entries.iter_mut().find(|(a, _)| *a == action) {
            Some(entry) => entry.1 = keys,
            None => {
                self.entries.push((action, keys));
                let order = |a: &Action| ACTIONS.iter().position(|x| x == a);
                self.entries.sort_by_key(|(a, _)| order(a));
            }
        }
    }

    /// 恢复命令 `action` 在方案中的键。
    pub fn reset(&mut self, action: Action) {
        self.entries.retain(|(a, _)| *a != action);
    }

    /// 从命令 `action` 改过的键中去掉与 `keys` 抢同一个键的；没改过该命令时不变。
    pub fn release(&mut self, action: Action, keys: Keys) {
        if let Some((_, list)) = self.entries.iter_mut().find(|(a, _)| *a == action) {
            list.retain(|&other| !clashes(other, keys));
        }
    }

    /// 返回方案 `preset` 在 `os` 上的绑定表合入改过的键之后的结果，生效范围小的绑定在前。
    ///
    /// 改过的命令在方案中的各生效范围内使用新键；方案中没有该命令时取另一套方案中的范围，
    /// 都没有时在所有区域生效。范围重叠的同一个键，以及与新键的第一段相同的单键或组合键，
    /// 都从其他命令上去掉；两个改过的命令抢同一个键时，排在后面的命令得到该键。
    pub fn apply(&self, preset: Preset, os: Os) -> Vec<Binding> {
        let base = preset.bindings(os);
        let mut others: Option<Vec<Vec<Binding>>> = None;
        let mut table = base.clone();
        let mut added: Vec<Binding> = Vec::new();
        for (action, keys) in &self.entries {
            let scopes = scopes(*action, &base, || {
                others.get_or_insert_with(|| Preset::ALL.map(|p| p.bindings(os)).to_vec())
            });
            table.retain(|b| b.action != *action);
            for &(first, second) in keys {
                for &when in &scopes {
                    let taken = |b: &Binding| {
                        b.when.overlaps(when) && clashes((b.first, b.second), (first, second))
                    };
                    table.retain(|b| !taken(b));
                    added.retain(|b| b.action == *action || !taken(b));
                    added.push(Binding {
                        action: *action,
                        first,
                        second,
                        when,
                    });
                }
            }
        }
        added.append(&mut table);
        added.sort_by_key(|b| b.when.breadth());
        added
    }
}

/// 按配置 `config` 中的键位方案和改过的快捷键安装键位表（见 [`install`]），返回读入时的问题。
pub fn install_from(config: &winc_config::Config) -> Vec<String> {
    let preset = (config.keymap.as_deref())
        .and_then(Preset::from_key)
        .unwrap_or_default();
    let (custom, problems) = Custom::from_config(&config.shortcuts);
    install(preset, &custom);
    problems
}

/// 按键 `a` 与 `b` 是否抢同一个键：按键相同，或者一方是单键、另一方是以它开头的组合键。
pub fn clashes(a: Keys, b: Keys) -> bool {
    a.0 == b.0 && (a.1 == b.1 || a.1.is_none() || b.1.is_none())
}

/// 返回命令 `action` 的生效范围：取自 `table`，其中没有时取自 `presets` 返回的各方案的表，
/// 都没有时为所有区域。
fn scopes<'a>(
    action: Action,
    table: &[Binding],
    presets: impl FnOnce() -> &'a mut Vec<Vec<Binding>>,
) -> Vec<When> {
    let collect = |table: &[Binding]| {
        let mut scopes: Vec<When> = Vec::new();
        for b in table.iter().filter(|b| b.action == action) {
            if !scopes.contains(&b.when) {
                scopes.push(b.when);
            }
        }
        scopes
    };
    let own = collect(table);
    if !own.is_empty() {
        return own;
    }
    (presets().iter())
        .map(|table| collect(table))
        .find(|scopes| !scopes.is_empty())
        .unwrap_or_else(|| vec![When::Global])
}
