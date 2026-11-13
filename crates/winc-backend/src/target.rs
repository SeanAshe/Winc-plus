//! 连接目标：传给 `ssh` 的主机写法。

#[cfg(test)]
mod tests;

/// 连接目标的最大长度（字节）：虚拟路径的第一段是固定前缀加连接目标，文件名一段最长 255 字节，
/// 取 200 为前缀留出余量；域名最长 253 字节，超过 200 字节的目标需要在 `~/.ssh/config` 中起别名。
const MAX_LEN: usize = 200;

/// 连接目标：`ssh` 的主机，可以是 `~/.ssh/config` 中的别名、主机名或 `用户@主机`，其后可以有
/// `:端口`。
///
/// 主机部分只接受字母、数字和 `.`、`_`、`-`、`@`，且不以 `-` 开头：它原样作为 `ssh` 的参数和
/// 虚拟路径的一段，以 `-` 开头会被 `ssh` 当作选项，其余字符在 shell 或路径中有特殊含义。端口是
/// 1 到 65535 的十进制数，写法不带前导的 0。跳板机和 IPv6 地址在 `~/.ssh/config` 中配置。
///
/// 端口是目标的一部分：同一主机的不同端口是不同的目标，各有各的连接。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target(String);

impl Target {
    /// 校验并创建连接目标；`text` 不符合要求时返回 `None`。
    pub fn new(text: &str) -> Option<Self> {
        let (host, port) = match text.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (text, None),
        };
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@');
        let valid = !host.is_empty()
            && text.len() <= MAX_LEN
            && !host.starts_with('-')
            && host.chars().all(allowed)
            && port.is_none_or(|port| {
                // 只有一种写法：不带符号和前导的 0，同一个端口不会成为两个目标。
                port.parse::<u16>()
                    .is_ok_and(|n| n != 0 && n.to_string() == port)
            });
        valid.then(|| Self(text.to_owned()))
    }

    /// 由主机 `host` 和端口 `port` 创建连接目标；`port` 为 `None` 时用 `ssh` 的默认端口。
    pub fn with_port(host: &str, port: Option<u16>) -> Option<Self> {
        match port {
            Some(port) => Self::new(&format!("{host}:{port}")),
            None => Self::new(host).filter(|target| target.port().is_none()),
        }
    }

    /// 返回连接目标的文本：主机，以及有端口时其后的 `:端口`。用于显示和虚拟路径。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 返回主机部分，可直接作为 `ssh` 的主机参数。
    pub fn host(&self) -> &str {
        self.0.split_once(':').map_or(&self.0, |(host, _)| host)
    }

    /// 返回端口；没有指定时为 `None`，由 `ssh` 按用户的配置决定。
    pub fn port(&self) -> Option<u16> {
        let (_, port) = self.0.split_once(':')?;
        port.parse().ok()
    }
}
