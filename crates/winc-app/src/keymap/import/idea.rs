//! IDEA 系列编辑器的键位方案：配置目录中的 `options/keymap.xml` 记着当前方案的名字，用户自定的
//! 方案在 `keymaps` 目录中，各自写明继承的方案和改过键的操作。
//!
//! 自定的方案中列出的操作整组取代所继承方案中的键，没有键的操作表示去掉了全部键。

use std::path::Path;

use winc_i18n::t;
use winc_preview::xml::{Token, Tokens, attribute};

use super::*;

/// 继承链上自定方案的层数上限，超过时视为循环继承。
const MAX_DEPTH: usize = 16;

/// 自定方案中一个操作的 id 和它的各个键：第一段和组合键的第二段（键盘按键的写法）。
type Shortcuts = (String, Vec<(String, Option<String>)>);

/// 一个自定的方案：继承的方案名，以及改过键的各操作。
struct Keymap {
    parent: String,
    actions: Vec<Shortcuts>,
}

/// 读取配置目录 `dir` 中当前使用的键位方案；按键写法取 `os` 上的。
pub(super) fn read(dir: &Path, os: Os) -> Result<Imported, String> {
    let Some(mut name) = active_keymap(dir, os) else {
        return Ok(Imported::default());
    };
    // 从当前方案沿继承链往上，直到内置的方案。
    let mut chain = Vec::new();
    let preset = loop {
        match builtin(&name) {
            Some(Some(preset)) => break preset,
            Some(None) => return Err(t!("keymap-import-unsupported", name = name)),
            None => {}
        }
        if chain.len() >= MAX_DEPTH {
            return Err(t!("keymap-import-unsupported", name = name));
        }
        let Some(keymap) = custom_keymap(dir, &name) else {
            return Err(t!("keymap-import-missing", name = name));
        };
        name = keymap.parent.clone();
        chain.push(keymap);
    };
    let mut custom = Custom::default();
    let mut skipped = 0;
    // 继承的方案先合入，下层方案改过的操作取代上层的。同一层中对应同一个命令的几个操作，
    // 键合在一起。
    for keymap in chain.iter().rev() {
        let mut layer: Vec<(Action, Vec<Keys>)> = Vec::new();
        for (id, shortcuts) in &keymap.actions {
            let Some(action) = ids::from_idea(id) else {
                skipped += 1;
                continue;
            };
            let index = match layer.iter().position(|(a, _)| *a == action) {
                Some(index) => index,
                None => {
                    layer.push((action, Vec::new()));
                    layer.len() - 1
                }
            };
            let keys = &mut layer[index].1;
            for (first, second) in shortcuts {
                let first = stroke(first);
                let second = second.as_deref().map(stroke);
                let parsed = match (first, second) {
                    (Some(first), None) => (first, None),
                    (Some(first), Some(Some(second))) => (first, Some(second)),
                    _ => {
                        skipped += 1;
                        continue;
                    }
                };
                if !keys.contains(&parsed) {
                    keys.push(parsed);
                }
            }
        }
        for (action, keys) in layer {
            custom.set(action, keys);
        }
    }
    Ok(Imported {
        preset,
        custom,
        skipped,
    })
}

/// 返回内置方案 `name` 对应的键位方案：IDEA 的默认方案为 [`Preset::Idea`]，VS Code 方案为
/// [`Preset::VsCode`]，其他内置方案为 `Some(None)`；不是内置方案时返回 `None`。
fn builtin(name: &str) -> Option<Option<Preset>> {
    match name {
        "$default" | "Mac OS X 10.5+" | "Mac OS X" | "Default for XWin" | "Default for GNOME"
        | "Default for KDE" => Some(Some(Preset::Idea)),
        "VSCode" | "VSCode OSX" => Some(Some(Preset::VsCode)),
        "Eclipse"
        | "Eclipse (Mac OS X)"
        | "Emacs"
        | "NetBeans 6.5"
        | "Sublime Text"
        | "Sublime Text (Mac OS X)"
        | "Visual Studio"
        | "Visual Studio OSX"
        | "Xcode"
        | "ReSharper"
        | "ReSharper OSX"
        | "macOS System Shortcuts" => Some(None),
        _ => None,
    }
}

/// 返回配置目录 `dir` 中记录的当前方案名；没有记录时返回 `None`（使用默认方案）。
///
/// 较新的版本按系统分目录记录（如 `options/mac/keymap.xml`），较旧的版本记在
/// `options/keymap.xml`。
fn active_keymap(dir: &Path, os: Os) -> Option<String> {
    let system = match os {
        Os::Mac => "mac",
        Os::Windows => "windows",
        Os::Linux => "linux",
    };
    let options = dir.join("options");
    [
        options.join(system).join("keymap.xml"),
        options.join("keymap.xml"),
    ]
    .iter()
    .find_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        Tokens::new(&text).find_map(|token| match token {
            Token::Open {
                name: "active_keymap",
                attributes,
                ..
            } => attribute(attributes, "name").map(|name| name.into_owned()),
            _ => None,
        })
    })
}

/// 读取配置目录 `dir` 的 `keymaps` 中名为 `name` 的自定方案；找不到或读不了时返回 `None`。
fn custom_keymap(dir: &Path, name: &str) -> Option<Keymap> {
    let entries = std::fs::read_dir(dir.join("keymaps")).ok()?;
    entries.flatten().find_map(|entry| {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "xml") {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        parse_keymap(&text)
            .filter(|(own, _)| own == name)
            .map(|(_, keymap)| keymap)
    })
}

/// 解析自定方案的 XML，返回方案名和内容。
fn parse_keymap(text: &str) -> Option<(String, Keymap)> {
    let mut tokens = Tokens::new(text);
    let (name, parent) = tokens.find_map(|token| match token {
        Token::Open {
            name: "keymap",
            attributes,
            ..
        } => Some((
            attribute(attributes, "name")?.into_owned(),
            attribute(attributes, "parent")?.into_owned(),
        )),
        _ => None,
    })?;
    let mut actions: Vec<Shortcuts> = Vec::new();
    for token in tokens {
        let Token::Open {
            name: tag,
            attributes,
            ..
        } = token
        else {
            continue;
        };
        match tag {
            "action" => {
                if let Some(id) = attribute(attributes, "id") {
                    actions.push((id.into_owned(), Vec::new()));
                }
            }
            "keyboard-shortcut" => {
                let first = attribute(attributes, "first-keystroke");
                let second = attribute(attributes, "second-keystroke");
                if let (Some((_, shortcuts)), Some(first)) = (actions.last_mut(), first) {
                    shortcuts.push((first.into_owned(), second.map(|s| s.into_owned())));
                }
            }
            _ => {}
        }
    }
    Some((name, Keymap { parent, actions }))
}

/// 解析一段按键的写法，如 `meta shift A`、`control OPEN_BRACKET`；认不出时返回 `None`。
fn stroke(text: &str) -> Option<Stroke> {
    let mut mods = Mods::NONE;
    let mut key = None;
    for token in text.split_whitespace() {
        match token {
            "meta" => mods = mods | Mods::META,
            "control" | "ctrl" => mods = mods | Mods::CTRL,
            "alt" | "altGraph" => mods = mods | Mods::ALT,
            "shift" => mods = mods | Mods::SHIFT,
            "pressed" => {}
            name if key.is_none() => key = Some(key_code(name)?),
            _ => return None,
        }
    }
    Some(Stroke { key: key?, mods })
}

/// 返回 IDEA 写法中的键名 `name` 对应的键。
fn key_code(name: &str) -> Option<KeyCode> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c
            .is_ascii_alphanumeric()
            .then(|| KeyCode::Char(c.to_ascii_lowercase()));
    }
    if let Some(digit) = name.strip_prefix("NUMPAD").filter(|d| d.len() == 1) {
        return digit
            .chars()
            .next()
            .filter(char::is_ascii_digit)
            .map(KeyCode::Numpad);
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=12).contains(&n).then_some(KeyCode::F(n));
    }
    Some(match name {
        "ENTER" => KeyCode::Enter,
        "BACK_SPACE" => KeyCode::Backspace,
        "DELETE" => KeyCode::Delete,
        "TAB" => KeyCode::Tab,
        "SPACE" => KeyCode::Space,
        "ESCAPE" => KeyCode::Escape,
        "UP" => KeyCode::Up,
        "DOWN" => KeyCode::Down,
        "LEFT" => KeyCode::Left,
        "RIGHT" => KeyCode::Right,
        "PAGE_UP" => KeyCode::PageUp,
        "PAGE_DOWN" => KeyCode::PageDown,
        "HOME" => KeyCode::Home,
        "END" => KeyCode::End,
        "INSERT" => KeyCode::Insert,
        "OPEN_BRACKET" => KeyCode::Char('['),
        "CLOSE_BRACKET" => KeyCode::Char(']'),
        "SLASH" => KeyCode::Char('/'),
        "BACK_SLASH" => KeyCode::Char('\\'),
        "MINUS" => KeyCode::Char('-'),
        "EQUALS" => KeyCode::Char('='),
        "COMMA" => KeyCode::Char(','),
        "PERIOD" => KeyCode::Char('.'),
        "SEMICOLON" => KeyCode::Char(';'),
        "QUOTE" => KeyCode::Char('\''),
        "BACK_QUOTE" => KeyCode::Char('`'),
        "ADD" => KeyCode::Numpad('+'),
        "SUBTRACT" => KeyCode::Numpad('-'),
        "MULTIPLY" => KeyCode::Numpad('*'),
        "DIVIDE" => KeyCode::Numpad('/'),
        "DECIMAL" => KeyCode::Numpad('.'),
        _ => return None,
    })
}
