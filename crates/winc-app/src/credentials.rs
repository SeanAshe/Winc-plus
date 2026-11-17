//! Git 的 HTTP(S) 凭据的保存位置：操作系统的凭据存储，不写入文件。
//!
//! macOS 为钥匙串，Windows 为凭据管理器，其他 Unix 平台为 Secret Service（D-Bus）。条目的名称是
//! `协议://主机` 或 `协议://用户名@主机`，内容是用户名、换行符和密码。测试中使用进程内的表，
//! 不访问系统的凭据存储。

#[cfg(all(target_os = "macos", not(test)))]
#[path = "credentials/macos.rs"]
mod imp;
#[cfg(all(windows, not(test)))]
#[path = "credentials/windows.rs"]
mod imp;
#[cfg(all(unix, not(target_os = "macos"), not(test)))]
#[path = "credentials/secret_service.rs"]
mod imp;
#[cfg(any(test, not(any(unix, windows))))]
mod imp {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, PoisonError};

    static ENTRIES: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

    fn entries() -> std::sync::MutexGuard<'static, BTreeMap<String, String>> {
        ENTRIES.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn available() -> bool {
        cfg!(test)
    }

    pub(super) fn get(name: &str) -> Option<String> {
        entries().get(name).cloned()
    }

    pub(super) fn set(name: &str, secret: &str) -> bool {
        entries().insert(name.to_owned(), secret.to_owned());
        true
    }

    pub(super) fn delete(name: &str) {
        entries().remove(name);
    }

    pub(super) fn names() -> Vec<String> {
        entries().keys().cloned().collect()
    }
}

/// 条目在系统凭据存储中所属的服务名，用户在系统的钥匙串工具中按此名称查找。
#[cfg_attr(any(test, not(any(unix, windows))), expect(dead_code))]
const SERVICE: &str = "Winc Git";

/// 一份已保存的凭据。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Credential {
    /// 用户名。
    pub username: String,
    /// 密码或访问令牌。
    pub password: String,
}

/// 是否有可用的系统凭据存储。
pub(crate) fn available() -> bool {
    imp::available()
}

/// 返回 `scheme://host` 的条目名；`username` 不为空时是该用户名专用的条目。
fn entry(origin: &str, username: Option<&str>) -> Option<String> {
    let (scheme, host) = origin.split_once("://")?;
    Some(match username {
        Some(user) => format!("{scheme}://{user}@{host}"),
        None => origin.to_owned(),
    })
}

/// 读取条目 `name`。
fn read(name: &str) -> Option<Credential> {
    let secret = imp::get(name)?;
    let (username, password) = secret.split_once('\n')?;
    Some(Credential {
        username: username.to_owned(),
        password: password.to_owned(),
    })
}

/// 返回为 `origin`（`scheme://host`）保存的凭据；`username` 不为空时只返回该用户名的凭据。
pub(crate) fn find(origin: &str, username: Option<&str>) -> Option<Credential> {
    let found = read(&entry(origin, username)?)?;
    username
        .is_none_or(|user| user == found.username)
        .then_some(found)
}

/// 保存 `origin` 的凭据，返回是否成功。
///
/// 写入两个条目：该用户名专用的条目，以及地址不含用户名时使用的条目。
pub(crate) fn save(origin: &str, credential: &Credential) -> bool {
    // 用户名含换行符时无法按「用户名、换行符、密码」存取。
    if credential.username.contains('\n') {
        return false;
    }
    let (Some(own), Some(shared)) = (
        entry(origin, Some(&credential.username)),
        entry(origin, None),
    ) else {
        return false;
    };
    let secret = format!("{}\n{}", credential.username, credential.password);
    let own = imp::set(&own, &secret);
    imp::set(&shared, &secret) && own
}

/// 删除 `origin` 的凭据；`username` 不为空时只删除该用户名的凭据。
pub(crate) fn forget(origin: &str, username: Option<&str>) {
    let Some(shared) = entry(origin, None) else {
        return;
    };
    if let Some(user) = username {
        if let Some(name) = entry(origin, Some(user)) {
            imp::delete(&name);
        }
        if read(&shared).is_some_and(|found| found.username != user) {
            return;
        }
    }
    imp::delete(&shared);
}

/// 删除全部已保存的凭据，返回涉及的地址数（个）。
pub(crate) fn forget_all() -> usize {
    let names = imp::names();
    for name in &names {
        imp::delete(name);
    }
    hosts(&names)
}

/// 返回条目 `names` 涉及的地址数：同一地址的两个条目只计一次。
fn hosts(names: &[String]) -> usize {
    let mut hosts: Vec<&str> = names
        .iter()
        .map(|name| {
            name.rsplit_once('@')
                .map_or(name.as_str(), |(_, host)| host)
        })
        .map(|name| name.rsplit_once("://").map_or(name, |(_, host)| host))
        .collect();
    hosts.sort_unstable();
    hosts.dedup();
    hosts.len()
}

#[cfg(test)]
mod tests;
