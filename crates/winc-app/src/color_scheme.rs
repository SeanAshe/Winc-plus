//! 经 XDG 桌面门户的 `org.freedesktop.portal.Settings` 读取并跟随系统的深浅色外观（macOS 以外的
//! Unix 平台）。

use std::sync::{Condvar, Mutex};
use std::time::Duration;

use winit::window::Theme;
use zbus::blocking::proxy::SignalIterator;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

/// 外观设置所在的命名空间。
const NAMESPACE: &str = "org.freedesktop.appearance";
/// 外观的设置项。
const KEY: &str = "color-scheme";

/// [`current`] 等待首次读取的最长时间。
const FIRST_READ: Duration = Duration::from_millis(100);

/// 门户的回答：还没读到时为 `None`；读到后为 `Some(外观)`，取不到时外观为 `None`。
static ANSWER: Mutex<Option<Option<Theme>>> = Mutex::new(None);
/// 首次读到时唤醒等待中的 [`current`]。
static ANSWERED: Condvar = Condvar::new();

/// 在后台读取系统的外观并等待它变化，每次读到都以该外观调用 `changed`；建窗口之前调用一次。
pub fn watch(changed: impl Fn(Theme) + Send + 'static) {
    std::thread::spawn(move || {
        let Ok((settings, changes)) = subscribe() else {
            publish(None);
            return;
        };
        let theme = read(&settings);
        publish(theme);
        if let Some(theme) = theme {
            changed(theme);
        }
        for message in changes {
            let body = message.body();
            let Ok((_, _, value)) = body.deserialize::<(&str, &str, Value<'_>)>() else {
                continue;
            };
            if let Some(theme) = theme_of(&value) {
                publish(Some(theme));
                changed(theme);
            }
        }
    });
}

/// 系统当前的外观，取不到时返回 `None`；首次读取最多等待 [`FIRST_READ`]，之后不再等待。
pub fn current() -> Option<Theme> {
    let answer = ANSWER.lock().ok()?;
    let (mut answer, _) = ANSWERED
        .wait_timeout_while(answer, FIRST_READ, |answer| answer.is_none())
        .ok()?;
    *answer.get_or_insert(None)
}

/// 记下门户的回答，唤醒等待中的 [`current`]。
fn publish(theme: Option<Theme>) {
    if let Ok(mut answer) = ANSWER.lock() {
        *answer = Some(theme);
    }
    ANSWERED.notify_all();
}

/// 连接会话总线，订阅外观的变化；在读取之前调用。
fn subscribe() -> zbus::Result<(Proxy<'static>, SignalIterator<'static>)> {
    let connection = Connection::session()?;
    let settings = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )?;
    let changes =
        settings.receive_signal_with_args("SettingChanged", &[(0, NAMESPACE), (1, KEY)])?;
    Ok((settings, changes))
}

/// 读取门户当前的外观；门户不可用或没有该设置时返回 `None`。
fn read(settings: &Proxy<'_>) -> Option<Theme> {
    let value: OwnedValue = settings.call("Read", &(NAMESPACE, KEY)).ok()?;
    theme_of(&value)
}

/// `color-scheme` 的取值 `value`（可以套着变体）对应的外观：只有 1 是深色；不是 `u32` 时返回
/// `None`。
fn theme_of(value: &Value<'_>) -> Option<Theme> {
    match value {
        Value::Value(inner) => theme_of(inner),
        Value::U32(1) => Some(Theme::Dark),
        Value::U32(_) => Some(Theme::Light),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
