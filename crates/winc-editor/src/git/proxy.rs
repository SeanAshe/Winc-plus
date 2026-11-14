//! 让 `git` 跟随系统的代理设置。
//!
//! `git` 自己只认 `http.proxy` 配置和代理的环境变量，不读系统设置里的代理；从图形界面启动的应用
//! 也常常没有这些环境变量。这里在运行会访问网络的 `git` 命令前读出系统的代理，命令的环境里没有
//! 代理变量时替它设上。`git` 的配置优先于环境变量，用户配置过 `http.proxy` 的仓库不受影响。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：读取本机的系统代理设置"
)]

use std::ffi::OsStr;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::STATUS_TIMEOUT;
use crate::tools::shell::process;

/// 是否跟随系统代理，见 [`follow_system_proxy`]。
static FOLLOW: AtomicBool = AtomicBool::new(true);

/// 设定 `git` 是否跟随系统的代理设置（默认跟随）。
pub fn follow_system_proxy(follow: bool) {
    FOLLOW.store(follow, Ordering::Relaxed);
}

/// 系统的代理设置中 `git` 用得上的部分。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct SystemProxy {
    /// 访问 `http://` 地址用的代理，写作 `主机:端口`。
    pub http: Option<String>,
    /// 访问 `https://` 地址用的代理，写作 `主机:端口`。
    pub https: Option<String>,
    /// SOCKS 代理，写作 `主机:端口`。
    pub socks: Option<String>,
    /// 不经过代理的主机。
    pub bypass: Vec<String>,
}

impl SystemProxy {
    /// 返回要为 `git` 设上的环境变量；没有可用的代理时为空。
    ///
    /// 系统里的 HTTP、HTTPS 代理都是 HTTP 代理（只是分别用于两种地址），地址写 `http://`。只有
    /// SOCKS 代理时用 `socks5h://`，域名交给代理解析。
    pub fn vars(&self) -> Vec<(&'static str, String)> {
        let mut vars = Vec::new();
        if let Some(proxy) = &self.http {
            vars.push(("http_proxy", format!("http://{proxy}")));
        }
        if let Some(proxy) = &self.https {
            vars.push(("https_proxy", format!("http://{proxy}")));
        }
        if vars.is_empty()
            && let Some(proxy) = &self.socks
        {
            vars.push(("all_proxy", format!("socks5h://{proxy}")));
        }
        let bypass = no_proxy(&self.bypass);
        if !vars.is_empty() && !bypass.is_empty() {
            vars.push(("no_proxy", bypass));
        }
        vars
    }
}

/// 把系统的例外列表换成 `no_proxy` 的写法：逗号分隔的主机名、域名后缀或网段。
///
/// `*.example.com`、`*example.com` 去掉通配符成为后缀；地址前缀 `10.*`、`192.168.*` 和不完整的
/// 网段 `169.254/16` 补成完整的网段（见 [`network`]）。写不成 `no_proxy` 的条目（`172.2*`、
/// Windows 的 `<local>`）略去，这些地址仍经过代理。
fn no_proxy(bypass: &[String]) -> String {
    let hosts = bypass.iter().filter_map(|entry| {
        let entry = entry.trim();
        if let Some(network) = network(entry) {
            return Some(network);
        }
        let host = entry.trim_start_matches('*');
        let plain = !host.is_empty() && !host.contains(['*', '/', '<']);
        plain.then(|| host.to_owned())
    });
    hosts.collect::<Vec<_>>().join(",")
}

/// 把写成地址前缀的例外换成网段：`10.*` 是 `10.0.0.0/8`，`192.168.*` 是 `192.168.0.0/16`，
/// `169.254/16` 是 `169.254.0.0/16`。不是这两种写法时返回 `None`。
fn network(entry: &str) -> Option<String> {
    let (prefix, bits) = match entry.split_once('/') {
        Some((prefix, bits)) => (prefix, Some(bits.parse::<u8>().ok()?)),
        None => (entry.strip_suffix(".*")?, None),
    };
    let octets: Vec<u8> = (prefix.split('.').map(str::parse))
        .collect::<Result<_, _>>()
        .ok()?;
    if octets.is_empty() || octets.len() > 4 || (bits.is_none() && octets.len() == 4) {
        return None;
    }
    let bits = bits.unwrap_or(8 * octets.len() as u8);
    let mut address = [0u8; 4];
    address[..octets.len()].copy_from_slice(&octets);
    let [a, b, c, d] = address;
    Some(format!("{a}.{b}.{c}.{d}/{bits}"))
}

/// 解析 macOS `scutil --proxy` 的输出。
pub(super) fn parse_scutil(text: &str) -> SystemProxy {
    let mut proxy = SystemProxy::default();
    let mut values = std::collections::HashMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(" : ") else {
            continue;
        };
        // 例外列表的各项以序号为键。
        if key.parse::<u32>().is_ok() {
            proxy.bypass.push(value.to_owned());
        } else {
            values.insert(key, value);
        }
    }
    let read = |name: &str| {
        let on = values.get(format!("{name}Enable").as_str()) == Some(&"1");
        let host = values.get(format!("{name}Proxy").as_str()).filter(|_| on)?;
        let port = values.get(format!("{name}Port").as_str())?;
        Some(format!("{host}:{port}"))
    };
    proxy.http = read("HTTP");
    proxy.https = read("HTTPS");
    proxy.socks = read("SOCKS");
    proxy
}

/// 解析 Windows 注册表「Internet Settings」一项的 `reg query` 输出。
///
/// `ProxyServer` 写作 `主机:端口`（各种地址都用它），或者 `http=主机:端口;https=…;socks=…`。
pub(super) fn parse_registry(text: &str) -> SystemProxy {
    let value = |name: &str| {
        text.lines().find_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next()? == name).then(|| parts.nth(1).unwrap_or_default().to_owned())
        })
    };
    let mut proxy = SystemProxy::default();
    let enabled = value("ProxyEnable").is_some_and(|v| v == "0x1");
    let Some(server) = value("ProxyServer").filter(|s| enabled && !s.is_empty()) else {
        return proxy;
    };
    if server.contains('=') {
        for part in server.split(';') {
            match part.split_once('=') {
                Some(("http", host)) => proxy.http = Some(host.to_owned()),
                Some(("https", host)) => proxy.https = Some(host.to_owned()),
                Some(("socks", host)) => proxy.socks = Some(host.to_owned()),
                _ => {}
            }
        }
    } else {
        proxy.http = Some(server.clone());
        proxy.https = Some(server);
    }
    let bypass = value("ProxyOverride").unwrap_or_default();
    proxy.bypass = bypass.split(';').map(str::to_owned).collect();
    proxy
}

/// 读出系统现在的代理设置；读不到时为 `None`。Linux 上没有统一的系统代理，约定用环境变量，
/// 不另外读取。
///
/// 经由 [`process::run`] 运行：Windows 上不弹出控制台窗口，卡住时与读取状态同样按时限结束，
/// 此时不设代理，命令照常运行。
fn system_proxy() -> Option<SystemProxy> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("scutil");
        command.arg("--proxy");
        command
    } else if cfg!(windows) {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
        let mut command = Command::new("reg");
        command.args(["query", key]);
        command
    } else {
        return None;
    };
    let cancel = process::Cancel::default();
    let output = process::run(&mut command, Vec::new(), STATUS_TIMEOUT, &cancel).ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Some(if cfg!(windows) {
        parse_registry(&text)
    } else {
        parse_scutil(&text)
    })
}

/// 代理的环境变量；`git` 用的库大小写两种写法都认。
/// 会访问网络、需要代理的 `git` 子命令。
const ONLINE: [&str; 6] = ["fetch", "pull", "push", "clone", "ls-remote", "remote"];

/// `git args` 的子命令是否会访问网络（见 [`ONLINE`]）：跳过前面的 `-c 键=值` 和其他选项，看第一个
/// 子命令。
pub(super) fn online<'a>(args: impl IntoIterator<Item = &'a OsStr>) -> bool {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "-c" {
            args.next();
            continue;
        }
        if arg.to_string_lossy().starts_with('-') {
            continue;
        }
        return ONLINE.iter().any(|command| arg == *command);
    }
    false
}

/// 读到的系统代理保留多久：几个仓库依次推送、获取时只读一次；改了系统设置以后很快就用上。
const PROXY_FRESH: Duration = Duration::from_secs(10);

/// 最近一次读到的系统代理和读取的时刻。
static CACHED: Mutex<Option<(Instant, SystemProxy)>> = Mutex::new(None);

/// 系统的代理设置，[`PROXY_FRESH`] 之内复用上次读到的。
fn cached_proxy() -> SystemProxy {
    let mut cached = CACHED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, proxy)) = cached.as_ref()
        && at.elapsed() < PROXY_FRESH
    {
        return proxy.clone();
    }
    let proxy = system_proxy().unwrap_or_default();
    *cached = Some((Instant::now(), proxy.clone()));
    proxy
}

const PROXY_VARS: [&str; 6] = [
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
];

/// 给 `command` 设上系统的代理：开着跟随、命令的环境里还没有代理变量时才设。
///
/// 每次运行前现读一遍：系统的代理随时会开关，不能记住上一次的结果。
pub(super) fn apply(command: &mut Command) {
    if !FOLLOW.load(Ordering::Relaxed) {
        return;
    }
    let configured = command.get_envs().any(|(name, value)| {
        let named = PROXY_VARS.iter().any(|var| name == *var);
        named && value.is_some_and(|value| !value.is_empty())
    });
    if configured {
        return;
    }
    for (name, value) in cached_proxy().vars() {
        command.env(name, value);
    }
}

#[cfg(test)]
mod tests;
