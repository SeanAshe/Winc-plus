//! 勾选式下的勾选记录。

use super::*;

/// 勾选式下的勾选：用户取消勾选的已跟踪文件、勾上的未跟踪文件，以及勾上的文件里取消勾选的
/// 各处改动。
///
/// 只能经由这里的方法改动，每改一次 [`Checks::version`] 加 1：组、目录的勾选框按它缓存（见
/// [`CommitRows::members_state`]），漏加时显示的勾选与实际的不一致。
#[derive(Debug, Default)]
pub(crate) struct Checks {
    /// 取消勾选的已跟踪文件；其余已跟踪的文件都勾着。
    excluded: HashSet<PathBuf>,
    /// 勾上的未跟踪文件；其余未跟踪的文件都不勾。
    included: HashSet<PathBuf>,
    /// 勾上的文件里取消勾选的各处改动：这些文件只提交其余的改动。
    skipped: HashMap<PathBuf, Vec<Skip>>,
    version: u64,
}

impl Checks {
    /// 勾选的版本：每改动一次加 1。
    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    /// 勾选之外决定勾选框的状态（暂存区）变了：按版本缓存的勾选框作废。
    pub(in crate::app::git) fn touch(&mut self) {
        self.version += 1;
    }

    /// 文件 `entry` 是否勾上：已跟踪的文件默认勾上，未跟踪的文件勾上后才算。
    pub(in crate::app::git) fn picked(&self, entry: &Entry) -> bool {
        if entry.untracked() {
            self.included.contains(&entry.path)
        } else {
            !self.excluded.contains(&entry.path)
        }
    }

    /// 文件 `path` 里取消勾选的各处改动。
    pub(in crate::app::git) fn skips(&self, path: &Path) -> &[Skip] {
        self.skipped.get(path).map_or(&[], Vec::as_slice)
    }

    /// 勾上或勾掉文件 `path`；之前取消勾选的各处改动随之作废。
    pub(in crate::app::git) fn set(&mut self, path: &Path, on: bool) {
        self.version += 1;
        self.skipped.remove(path);
        let (add, drop) = match on {
            true => (&mut self.included, &mut self.excluded),
            false => (&mut self.excluded, &mut self.included),
        };
        drop.remove(path);
        add.insert(path.to_path_buf());
    }

    /// 按别处记下的勾选恢复文件 `path`（`untracked` 是否未跟踪）：`on` 是否勾上，`skips` 是其中
    /// 取消勾选的各处改动。
    pub(in crate::app::git) fn restore(
        &mut self,
        path: &Path,
        (untracked, on): (bool, bool),
        skips: Option<Vec<Skip>>,
    ) {
        self.version += 1;
        self.included.remove(path);
        self.excluded.remove(path);
        match (on, untracked) {
            (true, true) => _ = self.included.insert(path.to_path_buf()),
            (false, false) => _ = self.excluded.insert(path.to_path_buf()),
            _ => {}
        }
        match skips.filter(|s| on && !s.is_empty()) {
            Some(skips) => _ = self.skipped.insert(path.to_path_buf(), skips),
            None => _ = self.skipped.remove(path),
        }
    }

    /// 记下文件 `path` 取消勾选的各处改动 `skips`。
    pub(in crate::app::git) fn set_skips(&mut self, path: &Path, skips: Vec<Skip>) {
        self.version += 1;
        self.skipped.insert(path.to_path_buf(), skips);
    }

    /// 取出文件 `path` 取消勾选的各处改动（换仓库时带到其他仓库的记录里）。
    pub(in crate::app::git) fn take_skips(&mut self, path: &Path) -> Option<Vec<Skip>> {
        self.version += 1;
        self.skipped.remove(path)
    }

    /// 取消勾选的各处改动全部作废（`HEAD` 变了，记下的行不再对应）。
    pub(in crate::app::git) fn clear_skips(&mut self) {
        self.version += 1;
        self.skipped.clear();
    }

    /// 回到默认：已跟踪的文件都勾上，未跟踪的都不勾，没有取消勾选的改动。
    pub(in crate::app::git) fn reset(&mut self) {
        self.version += 1;
        self.excluded.clear();
        self.included.clear();
        self.skipped.clear();
    }
}
