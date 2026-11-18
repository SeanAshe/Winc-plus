//! 快捷键的文字写法：键位方案、配置文件和导入共用。
//!
//! 一段按键是用 `+` 连接的修饰键和键名，如 `cmd+shift+p`；两段组合键之间用一个空格分隔，如
//! `ctrl+k ctrl+s`。只写一个修饰键表示单按该修饰键，`shift shift` 即连按两下 Shift。修饰键
//! 写作 `ctrl`、`shift`、`alt`、`cmd`（macOS 上的 ⌘，其他系统上的 Win / Super 键，也可以写
//! `meta`、`win`、`super`）。键名不区分大小写。

use super::*;

/// 返回一段按键的写法，如 `cmd+shift+p`；单按修饰键时只写该修饰键。
pub fn stroke_spec(stroke: Stroke, os: Os) -> String {
    let meta = if os == Os::Mac { "cmd" } else { "meta" };
    let mut parts: Vec<String> = [
        (Mods::CTRL, "ctrl"),
        (Mods::SHIFT, "shift"),
        (Mods::ALT, "alt"),
        (Mods::META, meta),
    ]
    .into_iter()
    .filter(|&(bit, _)| stroke.mods.has(bit))
    .map(|(_, name)| name.to_owned())
    .collect();
    let key = match stroke.key {
        KeyCode::Tap => return parts.join("+"),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Numpad(c) => match c {
            '+' => "numpad_add".to_owned(),
            '-' => "numpad_subtract".to_owned(),
            '*' => "numpad_multiply".to_owned(),
            '/' => "numpad_divide".to_owned(),
            '.' => "numpad_decimal".to_owned(),
            c => format!("numpad{c}"),
        },
        KeyCode::F(n) => format!("f{n}"),
        named => format!("{named:?}").to_lowercase(),
    };
    parts.push(key);
    parts.join("+")
}

/// 返回一条绑定的按键写法；两段组合键的两段之间用空格连接。
pub fn binding_spec(first: Stroke, second: Option<Stroke>, os: Os) -> String {
    match second {
        Some(second) => format!("{} {}", stroke_spec(first, os), stroke_spec(second, os)),
        None => stroke_spec(first, os),
    }
}

/// 解析按键写法 `text`，返回第一段和第二段（没有时为 `None`）；写法不对时返回 `None`。
pub fn parse_spec(text: &str) -> Option<(Stroke, Option<Stroke>)> {
    let mut parts = text.split_whitespace();
    let first = parse_stroke(parts.next()?)?;
    let second = parts.next().map(parse_stroke);
    if parts.next().is_some() {
        return None;
    }
    match second {
        Some(second) => Some((first, Some(second?))),
        None => Some((first, None)),
    }
}

/// 解析一段按键的写法，如 `cmd+shift+p` 或单按修饰键的 `shift`。
fn parse_stroke(text: &str) -> Option<Stroke> {
    let lower = text.to_ascii_lowercase();
    let tokens: Vec<&str> = split_tokens(&lower)?;
    let mut mods = Mods::NONE;
    let (last, rest) = tokens.split_last()?;
    for token in rest {
        mods = mods | modifier(token)?;
    }
    if let Some(bit) = modifier(last) {
        // 只有一个修饰键时表示单按它；修饰键之后缺少键名的写法不认。
        return rest.is_empty().then_some(Stroke {
            key: KeyCode::Tap,
            mods: bit,
        });
    }
    Some(Stroke {
        key: key_code(last)?,
        mods,
    })
}

/// 按 `+` 拆开一段按键；键名本身是 `+` 时（如 `ctrl++`）保留为最后一项。
fn split_tokens(text: &str) -> Option<Vec<&str>> {
    if let Some(head) = text.strip_suffix("++") {
        let mut tokens: Vec<&str> = head.split('+').collect();
        tokens.push("+");
        return tokens.iter().all(|t| !t.is_empty()).then_some(tokens);
    }
    let tokens: Vec<&str> = text.split('+').collect();
    tokens.iter().all(|t| !t.is_empty()).then_some(tokens)
}

/// 返回修饰键名 `name` 对应的修饰键；不是修饰键时返回 `None`。
fn modifier(name: &str) -> Option<Mods> {
    Some(match name {
        "ctrl" | "control" => Mods::CTRL,
        "shift" => Mods::SHIFT,
        "alt" | "option" | "opt" => Mods::ALT,
        "cmd" | "command" | "meta" | "win" | "super" => Mods::META,
        _ => return None,
    })
}

/// 返回键名 `name` 对应的键；不认识时返回 `None`。
fn key_code(name: &str) -> Option<KeyCode> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c.is_ascii_graphic().then_some(KeyCode::Char(c));
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=12).contains(&n).then_some(KeyCode::F(n));
    }
    if let Some(rest) = name.strip_prefix("numpad") {
        let rest = rest.trim_start_matches('_');
        return Some(KeyCode::Numpad(match rest {
            "add" => '+',
            "subtract" => '-',
            "multiply" => '*',
            "divide" => '/',
            "decimal" => '.',
            digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() => {
                char::from(digit.as_bytes()[0])
            }
            _ => return None,
        }));
    }
    Some(match name {
        "enter" | "return" => KeyCode::Enter,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Space,
        "escape" | "esc" => KeyCode::Escape,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "insert" | "ins" => KeyCode::Insert,
        _ => return None,
    })
}
