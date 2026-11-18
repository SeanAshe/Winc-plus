//! 远端目录挂载在本机的位置（配置项 `remote.localFolders`）：把远端的文件换算成本机上的路径，
//! 供「在文件管理器中显示」「用默认应用打开」这类只能交给本机程序的操作使用。
//!
//! Winc 不负责挂载：目录由用户用 SSHFS、Samba、网络驱动器等方式挂到本机，这里只按配置换算路径。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use winc_backend::{Location, Target};

use crate::app::App;

/// 远端的文件 `path` 在本机上的位置：按 `folders`（`remote.localFolders`，键为 `目标:/路径`，
/// 值为本机目录）换算。`path` 是本机路径、不在任何一项之内、或名字无法写成本机路径时返回
/// `None`；几项都包含它时取最深的一项。
///
/// 只换算路径，不访问本机的文件系统：挂载是否还在由调用方交给系统时处理。
pub(crate) fn local_folder(folders: &BTreeMap<String, String>, path: &Path) -> Option<PathBuf> {
    let at = winc_backend::locate(path)?;
    let mut best: Option<(usize, PathBuf)> = None;
    for (key, local) in folders {
        let Some((target, folder)) = parse_key(key) else {
            continue;
        };
        if best
            .as_ref()
            .is_some_and(|(depth, _)| *depth >= folder.len())
        {
            continue;
        }
        let root = Location {
            target,
            path: folder.as_bytes().to_vec(),
        };
        let Some(rest) = at.relative_to(&root) else {
            continue;
        };
        let Some(mapped) = join_local(local, rest) else {
            continue;
        };
        best = Some((folder.len(), mapped));
    }
    best.map(|(_, path)| path)
}

/// 拆开 `remote.localFolders` 的键 `目标:/路径`（读配置时已规范过）；目标无效时返回 `None`。
fn parse_key(key: &str) -> Option<(Target, &str)> {
    let at = key.find(":/")?;
    let target = Target::new(&key[..at])?;
    Some((target, &key[at + 1..]))
}

/// 本机目录 `local` 下的相对路径 `rest`（远端的写法，以 `/` 分段）；有一段不是 UTF-8、或含有
/// 本机路径里有特殊含义的字符时返回 `None`。
///
/// `local` 开头的 `~/` 指本机用户的主目录。Windows 上 `/` 换成 `\`，只写盘符（`Z:`）时指该盘的
/// 根目录，而不是该盘上的当前目录。
fn join_local(local: &str, rest: &[u8]) -> Option<PathBuf> {
    let mut base = expand_home(local);
    if cfg!(windows) {
        let mut text = base.to_string_lossy().replace('/', "\\");
        if text.len() == 2 && text.ends_with(':') {
            text.push('\\');
        }
        base = PathBuf::from(text);
    }
    for segment in rest.split(|&b| b == b'/').filter(|s| !s.is_empty()) {
        let name = std::str::from_utf8(segment).ok()?;
        if cfg!(windows) && !windows_name(name) {
            return None;
        }
        base.push(name);
    }
    Some(base)
}

/// 远端的名字 `name` 在 Windows 上是否原样表示同一个文件。
///
/// `\\`、`:` 会被当成分隔符或盘符，`<>"|?*` 不能出现在名字里，末尾的 `.` 和空格会被系统去掉，
/// `CON`、`NUL`、`COM1` 这类名字（带不带扩展名）指向设备：这些名字换算后会指向别的文件。
fn windows_name(name: &str) -> bool {
    const DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix)
            .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
    };
    !name.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        && !name.ends_with(['.', ' '])
        && !DEVICES.contains(&stem.as_str())
        && !numbered("COM")
        && !numbered("LPT")
}

/// 展开开头的 `~/`（本机用户的主目录）；没有主目录时原样返回。
fn expand_home(local: &str) -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    match (local.strip_prefix("~/").or(local.strip_prefix("~\\")), home) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(local),
    }
}

impl App {
    /// 远端的文件 `path` 在本机上的位置（按配置项 `remote.localFolders`），见 [`local_folder`]。
    pub(crate) fn local_folder(&self, path: &Path) -> Option<PathBuf> {
        local_folder(&self.config.remote_local_folders, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        (pairs.iter())
            .map(|(remote, local)| (remote.to_string(), local.to_string()))
            .collect()
    }

    fn remote(target: &str, path: &str) -> PathBuf {
        winc_backend::virtual_path(&Target::new(target).unwrap(), path.as_bytes())
    }

    #[test]
    fn files_under_a_mounted_folder_map_to_the_local_mount() {
        let local = if cfg!(windows) { "Z:" } else { "/mnt/devbox" };
        let map = folders(&[("devbox:/home/me", local)]);
        let expected = |rest: &[&str]| {
            let mut path = PathBuf::from(if cfg!(windows) { "Z:\\" } else { local });
            path.extend(rest);
            path
        };
        assert_eq!(
            local_folder(&map, &remote("devbox", "/home/me/proj/a.rs")),
            Some(expected(&["proj", "a.rs"]))
        );
        // 挂载的目录本身。
        assert_eq!(
            local_folder(&map, &remote("devbox", "/home/me")),
            Some(expected(&[]))
        );
        // 名字只是前缀相同、别的主机、同一主机的另一个端口、本机路径：都不换算。
        assert_eq!(local_folder(&map, &remote("devbox", "/home/meow/a")), None);
        assert_eq!(local_folder(&map, &remote("other", "/home/me/a")), None);
        assert_eq!(
            local_folder(&map, &remote("devbox:2222", "/home/me/a")),
            None
        );
        assert_eq!(local_folder(&map, Path::new("/home/me/a")), None);
    }

    /// Windows 上会被改写或指向设备的名字不换算。
    #[test]
    fn names_windows_would_change_are_refused() {
        for bad in [
            "a.", "a ", "a:b", "a\\b", "x?.txt", "CON", "nul.txt", "COM1", "lpt9.log",
        ] {
            assert!(!windows_name(bad), "{bad}");
        }
        for good in ["a.txt", "console", "COM10", ".env", "lpt"] {
            assert!(windows_name(good), "{good}");
        }
    }

    #[test]
    fn the_deepest_mount_wins_and_the_root_can_be_mounted() {
        let (outer, inner) = if cfg!(windows) {
            (r"Y:\devbox", r"Z:\proj")
        } else {
            ("/mnt/all", "/mnt/proj")
        };
        let map = folders(&[("devbox:2222:/", outer), ("devbox:2222:/srv/proj", inner)]);
        let file = remote("devbox:2222", "/srv/proj/src/main.rs");
        assert_eq!(
            local_folder(&map, &file),
            Some(Path::new(inner).join("src").join("main.rs"))
        );
        let other = remote("devbox:2222", "/etc/hosts");
        assert_eq!(
            local_folder(&map, &other),
            Some(Path::new(outer).join("etc").join("hosts"))
        );
        // 键里的目标无效时整项不用。
        let broken = folders(&[("-oProxyCommand=x:/", outer)]);
        assert_eq!(local_folder(&broken, &other), None);
    }
}
