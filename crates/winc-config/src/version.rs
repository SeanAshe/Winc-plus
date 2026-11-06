//! 更新版本的比较：按 `SemVer` 的预发布优先级规则排序，并兼容原有的纯数字分段版本。

use std::cmp::Ordering;

/// 比较两个版本号字符串，返回 `a` 相对 `b` 的先后。
///
/// 按 `SemVer` 的规则：先比较核心版本，正式版大于同一核心版本的预发布，忽略构建元数据；
/// 两段和四段的旧版数字版本仍按数字比较，缺少的核心字段补零。
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn parse(value: &str) -> (Vec<u64>, Option<String>) {
        let value = value.split('+').next().unwrap_or(value);
        let (core, pre) = value
            .split_once('-')
            .map_or((value, None), |(c, p)| (c, Some(p)));
        // `0.1.502_116` 与 `0.1.502.116` 是同一个版本：上游三段加上这个 fork 的数字。
        let core = match core.rsplit_once('_') {
            Some((head, tail))
                if !tail.is_empty() && tail.bytes().all(|c| c.is_ascii_digit()) =>
            {
                format!("{head}.{tail}")
            }
            _ => core.to_owned(),
        };
        let mut core: Vec<u64> = core
            .split('.')
            .map(|n| n.trim().parse().unwrap_or(0))
            .collect();
        core.resize(core.len().max(3), 0);
        (core, pre.map(str::to_owned))
    }
    let (ac, ap) = parse(a);
    let (bc, bp) = parse(b);
    ac.cmp(&bc).then_with(|| match (ap, bp) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => {
            let mut aa = a.split('.');
            let mut bb = b.split('.');
            loop {
                let (a, b) = match (aa.next(), bb.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, _) => return Ordering::Less,
                    (_, None) => return Ordering::Greater,
                    (Some(a), Some(b)) => (a, b),
                };
                let numeric = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
                let order = match (numeric(a), numeric(b)) {
                    (true, true) => {
                        let a = a.trim_start_matches('0');
                        let b = b.trim_start_matches('0');
                        a.len().cmp(&b.len()).then_with(|| a.cmp(b))
                    }
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => a.cmp(b),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests;
