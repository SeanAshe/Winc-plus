//! 连接目标：传给 `ssh` 的主机写法。

use std::borrow::Cow;

#[cfg(test)]
mod tests;

/// 连接目标的最大长度（字节）：虚拟路径的第一段是固定前缀加连接目标，文件名一段最长 255 字节，
/// 取 200 为前缀留出余量；域名最长 253 字节，超过 200 字节的目标需要在 `~/.ssh/config` 中起别名。
const MAX_LEN: usize = 200;

/// 连接目标：`ssh` 的主机，可以是 `~/.ssh/config` 中的别名、主机名或 `用户@主机`，其后可以有
/// `:端口`。IPv6 地址写在方括号中，可以带 `%接口名`：`用户@[fe80::1%eth0]:端口`。
///
/// 主机部分只接受字母、数字和 `.`、`_`、`-`、`@`，且不以 `-` 开头（它原样作为 `ssh` 的参数和
/// 虚拟路径的一段）。端口是 1 到 65535 的十进制数，不带前导的 0；同一主机的不同端口是不同的
/// 目标。跳板机在 `~/.ssh/config` 中配置。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target(String);

impl Target {
    /// 校验并创建连接目标；`text` 不符合要求时返回 `None`。
    pub fn new(text: &str) -> Option<Self> {
        let (host, port) = split(text);
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@');
        let named =
            |host: &str| !host.is_empty() && !host.starts_with('-') && host.chars().all(allowed);
        let host_valid = match host.split_once('[') {
            Some((user, address)) => {
                (user.is_empty() || user.ends_with('@') && named(user))
                    && address.strip_suffix(']').is_some_and(is_ipv6)
            }
            None => named(host),
        };
        let valid = host_valid
            && text.len() <= MAX_LEN
            && port.is_none_or(|port| {
                // 只有一种写法：不带符号和前导的 0，同一个端口不会成为两个目标。
                port.parse::<u16>()
                    .is_ok_and(|n| n != 0 && n.to_string() == port)
            });
        valid.then(|| Self(text.to_owned()))
    }

    /// 由主机 `host`（IPv6 地址可以不带方括号）和端口 `port` 创建连接目标；`port` 为 `None` 时
    /// 用 `ssh` 的默认端口。
    pub fn with_port(host: &str, port: Option<u16>) -> Option<Self> {
        let (user, address) = host.rsplit_once('@').map_or(("", host), |(user, address)| {
            (&host[..=user.len()], address)
        });
        let host = if is_ipv6(address) {
            Cow::Owned(format!("{user}[{address}]"))
        } else {
            Cow::Borrowed(host)
        };
        match port {
            Some(port) => Self::new(&format!("{host}:{port}")),
            None => Self::new(&host).filter(|target| target.port().is_none()),
        }
    }

    /// 返回连接目标的文本：主机，以及有端口时其后的 `:端口`。用于显示和虚拟路径。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 返回主机部分，可直接作为 `ssh` 的主机参数；IPv6 地址不带方括号。
    pub fn host(&self) -> Cow<'_, str> {
        let host = split(&self.0).0;
        if host.contains('[') {
            Cow::Owned(host.replace(['[', ']'], ""))
        } else {
            Cow::Borrowed(host)
        }
    }

    /// 返回端口；没有指定时为 `None`，由 `ssh` 按用户的配置决定。
    pub fn port(&self) -> Option<u16> {
        split(&self.0).1?.parse().ok()
    }
}

/// 把连接目标的文本 `text` 拆成主机和端口。
fn split(text: &str) -> (&str, Option<&str>) {
    let bracket = text.rfind(']').map_or(0, |at| at + 1);
    match text[bracket..].split_once(':') {
        Some((rest, port)) => (&text[..bracket + rest.len()], Some(port)),
        None => (text, None),
    }
}

/// `text` 是否是 IPv6 地址，可以带 `%接口名`。
fn is_ipv6(text: &str) -> bool {
    let (address, zone) = match text.split_once('%') {
        Some((address, zone)) => (address, Some(zone)),
        None => (text, None),
    };
    let named = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    address.parse::<std::net::Ipv6Addr>().is_ok()
        && zone.is_none_or(|zone| !zone.is_empty() && zone.chars().all(named))
}
