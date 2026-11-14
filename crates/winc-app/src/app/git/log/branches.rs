//! 日志左侧的分支面板：各行和分支选择器的相同（最近、本地、远程、标签各一组，分支名按 `/`
//! 分出目录，收起、展开的状态也共用）。点一个分支或标签只看它的提交，再点一次看全部；双击签出，
//! 右键是分支的各项操作。

use super::*;
pub(crate) use crate::app::git::branch::picker::BranchRow;
use crate::app::git::branch::picker::{BranchList, branch_rows, locals_key, without_recent};

/// 分支面板一组或一个目录展开、收起中的一帧（与提交视图里组、目录的动画相同）。
#[derive(Debug, Clone)]
pub(crate) struct PanelFold {
    /// 动画期间画的各行：展开时是展开以后的，收起时是收起之前的。
    pub rows: Vec<BranchRow>,
    /// 展开、收起的那一行在 `rows` 里的下标。
    pub parent: usize,
    /// 露出或收回的行数。
    pub count: usize,
    /// 是展开；`false` 是收起。
    pub expanding: bool,
    /// 这一段动画的时钟。
    pub(in crate::app) clock: crate::app::Clock,
}

impl PanelFold {
    /// 已露出的比例：0 是完全收起，1 是完全展开。
    pub fn reveal(&self) -> f32 {
        let eased = self.clock.progress();
        if self.expanding { eased } else { 1.0 - eased }
    }

    /// 还没露出来的高度（逻辑像素）。
    pub fn hidden(&self) -> f32 {
        (1.0 - self.reveal()) * self.count as f32 * BRANCH_ROW
    }
}

/// 选中一个提交以后过多久才去查所在分支。
///
/// 长于按住方向键时连续移动的间隔（系统默认的重复间隔不到 100 ms），连续移动时不查，停下
/// 以后很快就查。
pub(super) const BRANCHES_DELAY: Duration = Duration::from_millis(150);
/// 分支面板一行的高度（逻辑像素），与详情里文件行相同。
pub(crate) const BRANCH_ROW: f32 = LOG_FILE_ROW;

impl App {
    /// 在后台查找包含日志标签 `id` 选中的提交的分支（已经查到或正在查时不再查）；只查选中的，
    /// 不随详情预取：分支多、历史长时较慢，键盘连续移动时只查停下的那个。
    pub(super) fn fetch_log_branches(&mut self, id: DocumentId) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let view = self.editor.document(id).and_then(|d| d.log.as_deref());
        let Some(details) = view.and_then(|v| v.details.as_ref()) else {
            return;
        };
        let oid = details.oid.clone();
        if details.branches.is_some() || !self.git.log.branches_fetching.insert(oid.clone()) {
            return;
        }
        let sender = self.git.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let cancel = Cancel::default();
            let result = match tool {
                Ok(git) => git.containing_branches(&root, &oid, &cancel),
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Loaded::Branches {
                doc: id,
                oid,
                result,
            });
        });
    }

    /// 选中了日志标签 `id` 里的一个提交：停下 [`BRANCHES_DELAY`] 以后再查包含它的分支（见
    /// [`App::log_tick`]）。上次没查出来的这次重新查。
    pub(super) fn schedule_log_branches(&mut self, id: DocumentId) {
        let view = self.editor.document(id).and_then(|d| d.log.as_deref());
        if let Some(oid) = view.and_then(|v| v.details.as_ref()).map(|d| &d.oid) {
            self.git.log.branches_failed.remove(oid);
        }
        self.git.log.branches_due = Some((id, Instant::now() + BRANCHES_DELAY));
    }

    /// 分支面板里的各行：最上面是「全部分支」，其余与分支选择器（不搜索时）相同，只是没有
    /// 「最近」一组（见 [`without_recent`]）。
    ///
    /// 每帧都要用，分支很多（几千个远程分支）时分组、排序不便宜：按排它们用到的各项缓存，这几项
    /// 没变时直接用上次排好的。
    pub(crate) fn log_branch_rows(&self) -> Vec<BranchRow> {
        let git = &self.git;
        let mut cache = self.git.log.branch_rows.borrow_mut();
        // 和上次的直接比较，不先复制一份：复制几千个分支和重排差不多一样费时。
        if let Some(((b, r, f, t), rows)) = cache.as_ref()
            && *b == git.branches
            && *r == git.recent_branches
            && *f == git.branch_folders
            && *t == git.tags
        {
            return rows.clone();
        }
        let list = BranchList {
            branches: &git.branches,
            recent: &git.recent_branches,
            query: &[],
            text: "",
            toggled: &git.branch_folders,
            tags: &git.tags,
        };
        let mut rows = vec![BranchRow::All];
        rows.extend(without_recent(branch_rows(&list)));
        let key = (
            git.branches.clone(),
            git.recent_branches.clone(),
            git.branch_folders.clone(),
            git.tags.clone(),
        );
        *cache = Some((key, rows.clone()));
        rows
    }

    /// 分支面板第 `i` 行的分支（`GitState::branches` 里的）；不是分支的行为 `None`。
    fn log_branch_at(&self, i: usize) -> Option<&winc_editor::git::Branch> {
        match self.log_branch_rows().into_iter().nth(i)? {
            BranchRow::Branch { index, .. } => self.git.branches.get(index),
            _ => None,
        }
    }

    /// 双击了分支面板的第 `i` 行：是分支（不是当前分支）时签出，远程分支签出成同名的本地分支
    /// 并跟踪它，与分支选择器的「签出」相同。
    pub(in crate::app) fn checkout_log_branch_row(&mut self, i: usize) {
        let branch = self.log_branch_at(i).cloned();
        if let Some(branch) = branch.filter(|b| !b.current) {
            self.run_branch_op(crate::app::git::branch::BranchOp::Checkout, branch);
        }
    }

    /// 右键点在分支面板的一个分支上：返回它在分支列表（`GitState::branches`）里的下标，菜单
    /// 是分支选择器里同样的操作。
    pub(in crate::app) fn log_branch_context_target(&self, x: f32, y: f32) -> Option<usize> {
        if !self.log_active() {
            return None;
        }
        let LogTarget::BranchRow(i) = crate::layout::hit(&self.hits.log, x, y)? else {
            return None;
        };
        match self.log_branch_rows().into_iter().nth(i)? {
            BranchRow::Branch { index, .. } => Some(index),
            _ => None,
        }
    }

    /// 分支面板露出的比例：关着为 0，开着为 1，打开、关闭途中在两者之间。
    pub(crate) fn log_branches_shown(&self) -> f32 {
        let open = f32::from(u8::from(self.state.log_branches.unwrap_or(true)));
        let motion = self.git.log.branches_motion;
        motion.map_or(open, |(from, clock)| clock.between(from, open))
    }

    /// 显示或隐藏分支面板，面板从左边滑进、滑出，与侧栏开合的节奏相同；所有窗口共用，下次
    /// 启动沿用。
    pub(super) fn toggle_log_branches(&mut self) {
        let from = self.log_branches_shown();
        let shown = !self.state.log_branches.unwrap_or(true);
        self.update_state(|state| state.log_branches = Some(shown));
        let clock = self.clock(crate::app::animation::SIDEBAR_ANIMATION);
        self.git.log.branches_motion = clock.map(|clock| (from, clock));
        self.relayout();
    }

    /// 点了分支面板的第 `i` 行：「全部分支」不再按分支筛选；分支、标签只看它的提交，已经在看它
    /// 时改回全部，按住 `⌘` 点时加上或去掉它；组、目录收起或展开（和分支选择器共用）。
    pub(super) fn press_log_branch_row(&mut self, i: usize) {
        let Some(row) = self.log_branch_rows().into_iter().nth(i) else {
            return;
        };
        let name = match &row {
            BranchRow::Branch { index, .. } => {
                self.git.branches.get(*index).map(|b| b.name.clone())
            }
            BranchRow::Tag(index) => self.git.tags.get(*index).cloned(),
            _ => None,
        };
        let all = row == BranchRow::All;
        let fold = match row {
            BranchRow::Locals { recent, .. } => Some(locals_key(recent)),
            BranchRow::Remotes { .. } => Some((true, String::new())),
            BranchRow::Tags { .. } => Some((false, String::new())),
            BranchRow::Folder { remote, path, .. } => Some((remote, path)),
            _ => None,
        };
        if let Some(key) = fold {
            self.toggle_log_branch_fold(i, key);
        } else if let Some(name) = name {
            match self.command_held() {
                // 按住 `⌘` 点：在已选的分支里加上或去掉它，同时看几个分支。
                true => {
                    self.change_log_query(|q| match q.branches.iter().position(|b| *b == name) {
                        Some(at) => _ = q.branches.remove(at),
                        None => q.branches.push(name),
                    })
                }
                false => {
                    let same = self.log_branch_filter() == Some(name.as_str());
                    let chosen = (!same).then_some(name).into_iter().collect();
                    self.change_log_query(|q| q.branches = chosen);
                }
            }
        } else if all {
            self.change_log_query(|q| q.branches.clear());
        }
        self.request_redraw();
    }

    /// 收起或展开分支面板第 `parent` 行的组、目录（记法见 [`BranchList::toggled`]），子行从这一行
    /// 下方逐渐露出（或收回），下面的行随之移动；动态效果关闭时直接到位。
    fn toggle_log_branch_fold(&mut self, parent: usize, key: (bool, String)) {
        let before = self.log_branch_rows();
        if !self.git.branch_folders.remove(&key) {
            self.git.branch_folders.insert(key);
        }
        let after = self.log_branch_rows();
        let expanding = after.len() > before.len();
        let count = after.len().abs_diff(before.len());
        let rows = if expanding { after } else { before };
        self.git.log.branch_fold = (count > 0)
            .then(|| self.clock(crate::app::animation::TREE_ANIMATION))
            .flatten()
            .map(|clock| PanelFold {
                rows,
                parent,
                count,
                expanding,
                clock,
            });
    }
}
