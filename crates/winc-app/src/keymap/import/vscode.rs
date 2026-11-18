//! VS Code 用户目录中的 `keybindings.json`：在默认快捷键之上增加的键，以及命令名前加 `-` 去掉
//! 的键。

use std::path::Path;

use winc_config::Json;
use winc_i18n::t;

use super::*;

/// 读取用户目录 `dir` 中的 `keybindings.json`；没有该文件时只换用 VS Code 方案。
pub(super) fn read(dir: &Path, os: Os) -> Result<Imported, String> {
    let path = dir.join("keybindings.json");
    let shown = path.display().to_string();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Imported {
                preset: Preset::VsCode,
                ..Imported::default()
            });
        }
        Err(e) => {
            let error = e.to_string();
            return Err(t!("keymap-import-read-failed", path = shown, error = error));
        }
    };
    let entries = Json::parse_jsonc(&text).and_then(|(json, _)| match json {
        Json::Array(entries) => Some(entries),
        _ => None,
    });
    match entries {
        Some(entries) => Ok(parse(&entries, os)),
        None => Err(t!("keymap-import-invalid", path = shown)),
    }
}

/// 把 `keybindings.json` 的各项 `entries` 合入 VS Code 方案在 `os` 上的键。
fn parse(entries: &[Json], os: Os) -> Imported {
    let table = Preset::VsCode.bindings(os);
    // 改动过的命令、方案中的键和改动后的全部键。
    let mut changed: Vec<(Action, Vec<Keys>, Vec<Keys>)> = Vec::new();
    let mut skipped = 0;
    for entry in entries {
        let object = entry.as_object();
        let field = |name| object.and_then(|o| o.get(name)).and_then(Json::as_str);
        let Some(command) = field("command") else {
            skipped += 1;
            continue;
        };
        let (remove, id) = match command.strip_prefix('-') {
            Some(id) => (true, id),
            None => (false, command),
        };
        let Some(action) = ids::from_vscode(id) else {
            skipped += 1;
            continue;
        };
        let key = match field("key").map(parse_spec) {
            Some(Some(key)) => Some(key),
            Some(None) => {
                skipped += 1;
                continue;
            }
            None => None,
        };
        let index = match changed.iter().position(|(a, ..)| *a == action) {
            Some(index) => index,
            None => {
                let mut keys: Vec<Keys> = Vec::new();
                for b in table.iter().filter(|b| b.action == action) {
                    if !keys.contains(&(b.first, b.second)) {
                        keys.push((b.first, b.second));
                    }
                }
                changed.push((action, keys.clone(), keys));
                changed.len() - 1
            }
        };
        let keys = &mut changed[index].2;
        match (remove, key) {
            (true, Some(key)) => keys.retain(|k| *k != key),
            (true, None) => keys.clear(),
            (false, Some(key)) if !keys.contains(&key) => keys.push(key),
            (false, _) => {}
        }
    }
    // 增删之后与方案相同的命令不算改过。
    let mut custom = Custom::default();
    for (action, preset, keys) in changed {
        if keys != preset {
            custom.set(action, keys);
        }
    }
    Imported {
        preset: Preset::VsCode,
        custom,
        skipped,
    }
}
