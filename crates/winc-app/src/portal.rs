//! 选择文件、目录的面板（macOS 以外的 Unix 平台）：通过 XDG 桌面门户（xdg-desktop-portal）的
//! `org.freedesktop.portal.FileChooser` 接口，由桌面环境显示面板，沙盒中的应用也可使用。
//!
//! 调用是异步的：`OpenFile`、`SaveFile` 立即返回请求对象，用户选完以后该对象发出 `Response`
//! 信号。后台线程等待该信号，把结果交给回调。请求对象的路径按门户文档事先算出（连接的唯一名
//! 加 `handle_token`），先订阅信号再发起调用，因此不会错过回应。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// 面板的类型：打开或另存为。
pub enum Choose<'a> {
    /// 打开：`directory` 为真时只能选目录，否则只能选文件（门户不能同时选两种）；初始目录为
    /// `dir`，`None` 时由门户决定。
    Open {
        directory: bool,
        dir: Option<&'a Path>,
    },
    /// 另存为：默认文件名 `name`，初始目录为 `dir`。
    Save {
        name: &'a str,
        dir: Option<&'a Path>,
    },
}

/// 把目录 `dir` 写成门户要求的以 NUL 结尾的字节串。
fn folder_bytes(dir: &Path) -> Vec<u8> {
    let mut bytes = dir.as_os_str().as_encoded_bytes().to_vec();
    bytes.push(0);
    bytes
}

/// 在后台显示门户的面板，用户选完后以结果调用 `done`。
///
/// 结果为：选中时 `Ok(Some(路径))`；取消时 `Ok(None)`；门户不可用（没有会话总线、没有安装
/// xdg-desktop-portal 或其文件选择后端）时 `Err(None)`；其他错误为 `Err(Some(原因))`。
pub fn choose(
    choose: Choose<'_>,
    title: &str,
    done: impl FnOnce(Result<Option<PathBuf>, Option<String>>) + Send + 'static,
) {
    let request = Request::from(choose, title);
    std::thread::spawn(move || done(request.run()));
}

/// `error` 是否表示门户不存在：总线上没有该服务，或服务没有文件选择接口。
fn missing(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, ..) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownMethod"
        ),
        _ => false,
    }
}

/// 一次面板请求的参数。
struct Request {
    method: &'static str,
    title: String,
    directory: bool,
    name: Option<String>,
    dir: Option<Vec<u8>>,
}

impl Request {
    fn from(choose: Choose<'_>, title: &str) -> Request {
        match choose {
            Choose::Open { directory, dir } => Request {
                method: "OpenFile",
                title: title.to_owned(),
                directory,
                name: None,
                dir: dir.map(folder_bytes),
            },
            Choose::Save { name, dir } => Request {
                method: "SaveFile",
                title: title.to_owned(),
                directory: false,
                name: Some(name.to_owned()),
                dir: dir.map(folder_bytes),
            },
        }
    }

    /// 连接会话总线，发起调用，等待用户选择；错误的含义同 [`choose`]。
    fn run(self) -> Result<Option<PathBuf>, Option<String>> {
        // 连不上会话总线：没有桌面会话，也就没有门户。
        let connection = Connection::session().map_err(|_| None)?;
        self.ask(&connection)
            .map_err(|e| (!missing(&e)).then(|| e.to_string()))
    }

    /// 发起调用，等待用户选择。
    fn ask(&self, connection: &Connection) -> zbus::Result<Option<PathBuf>> {
        // 本进程内的请求编号：和进程号一起拼进 `handle_token`，同一进程的每次请求都不重复。
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let token = format!(
            "winc{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let sender = connection
            .unique_name()
            .map(|n| n.trim_start_matches(':').replace('.', "_"))
            .unwrap_or_default();
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let request = Proxy::new(
            connection,
            "org.freedesktop.portal.Desktop",
            path.as_str(),
            "org.freedesktop.portal.Request",
        )?;
        let mut responses = request.receive_signal("Response")?;

        let chooser = Proxy::new(
            connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.FileChooser",
        )?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("modal", Value::from(true));
        if self.directory {
            options.insert("directory", Value::from(true));
        }
        if let Some(name) = &self.name {
            options.insert("current_name", Value::from(name.as_str()));
        }
        if let Some(dir) = &self.dir {
            options.insert("current_folder", Value::from(dir.as_slice()));
        }
        // 第一个参数是父窗口的标识；不提供时面板不依附于窗口，仍然可以显示。
        let _: OwnedObjectPath = chooser.call(self.method, &("", self.title.as_str(), options))?;

        let Some(message) = responses.next() else {
            return Ok(None);
        };
        let (response, results): (u32, HashMap<String, OwnedValue>) =
            message.body().deserialize()?;
        // 0 表示用户确认，1 表示用户取消，2 表示因其他原因结束。
        if response != 0 {
            return Ok(None);
        }
        let uris = results
            .get("uris")
            .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default();
        Ok(uris.first().and_then(|uri| file_path(uri)))
    }
}

/// `file://` 网址对应的本地路径（百分号编码解码）；不是 `file://` 网址时返回 `None`。
fn file_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // 网址可以带主机名（`file://host/path`），本机路径从第一个 `/` 开始。
    let rest = &rest[rest.find('/')?..];
    let mut bytes = Vec::with_capacity(rest.len());
    let mut iter = rest.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hex = [iter.next()?, iter.next()?];
            let hex = std::str::from_utf8(&hex).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests;
