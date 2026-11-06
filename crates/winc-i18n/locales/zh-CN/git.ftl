## Git 命令行：查找、运行和读取结果

git-unavailable-not-installed = 未找到 Git
git-unavailable-no-developer-tools = 尚未安装命令行开发者工具
git-status-unreadable = 无法读取 git status 的输出
git-status-truncated = 仓库中有变化的文件过多，只显示了前 { $count } 个
git-push-no-remote = 此仓库未配置远程仓库
git-file-outside-repository = 文件不在 Git 仓库中
git-run-failed = Git 运行失败
git-run-spawn-failed = 无法运行 Git（{ $error }）
git-commit-partial-during-operation = { $operation }进行中，无法只提交部分更改
git-trust-config-unreadable = 无法读取仓库的配置
git-trust-required = 此仓库配置了在 Git 操作时自动运行的程序（{ $commands }），信任此仓库后才能执行此操作
git-list-separator = 、
git-worktree-copy-invalid-glob = git.worktreeCopy 中的「{ $glob }」有误：{ $error }
git-worktree-copy-failed = 无法复制「{ $path }」：{ $error }

## 进行中的操作

git-operation-merge = 合并
git-operation-rebase = 变基
git-operation-cherry-pick = 拣选
git-operation-revert = 还原

## 合并冲突的取舍

git-conflict-accept-current = 采用当前
git-conflict-accept-incoming = 采用传入
git-conflict-accept-both = 保留两者

## 分支菜单

git-branch-checkout = 签出
git-branch-checkout-new-worktree = 在新工作树中签出
git-branch-new-from = 基于此分支新建分支…
git-branch-merge-into-current = 合并到当前分支
git-branch-rebase-current-onto = 将当前分支变基到此分支
git-branch-delete = 删除分支…
git-branch-delete-confirm = 要删除分支「{ $name }」吗？
git-branch-delete-confirm-detail = 仅删除本地分支，远程分支不受影响。
git-branch-delete-confirm-button = 删除
git-branch-force-delete-confirm = 分支「{ $name }」尚未完全合并，仍要删除吗？
git-branch-force-delete-confirm-detail = 只属于此分支的提交可能会丢失。
git-branch-force-delete-confirm-button = 仍要删除
git-branch-compare-with-current = 与当前分支比较
git-stash-menu = 储藏（{ $count }）
git-stash-apply = 应用
git-stash-pop = 应用并删除
git-stash-drop = 删除…
git-stash-drop-confirm = 要删除储藏「{ $name }」吗？
git-stash-gone = 这条储藏已不存在
git-stash-drop-confirm-detail = 删除后，此储藏中的更改将无法恢复。
git-stash-drop-confirm-button = 删除

## 后台的 Git 操作：做的是什么，以及做完后的提示

git-op-fetch = 获取
git-op-pull = 拉取
git-op-push = 推送
git-op-sync = 同步
git-op-stash = 储藏更改
git-op-stash-apply = 应用储藏「{ $name }」
git-op-stash-pop = 应用并删除储藏「{ $name }」
git-op-continue-merge = 继续合并
git-op-continue-rebase = 继续变基
git-op-continue-cherry-pick = 继续拣选
git-op-continue-revert = 继续还原
git-op-abort-merge = 中止合并
git-op-abort-rebase = 中止变基
git-op-abort-cherry-pick = 中止拣选
git-op-abort-revert = 中止还原
git-op-checkout = 签出「{ $name }」
git-op-merge = 合并「{ $name }」
git-op-rebase-onto = 变基到「{ $name }」
git-op-delete-branch = 删除分支「{ $name }」
git-op-new-branch = 新建分支「{ $name }」
git-op-new-tag = 新建标签「{ $name }」
git-op-busy = 上一个 Git 操作尚未完成
git-op-done = 已{ $what }
git-op-done-summary = 已{ $what }：{ $summary }
git-op-done-in = 已在「{ $name }」中{ $what }
git-op-failed = 无法{ $what }：{ $error }
git-op-failed-in = 无法在「{ $name }」中{ $what }：{ $error }

## 工作树

git-worktree-detached = { $oid }（分离 HEAD）
git-worktree-missing = 目录已不存在
git-worktree-copied-ignored = 已复制 { $count } 个被忽略的文件
git-worktree-open-elsewhere = 工作树「{ $name }」已在另一个窗口中打开
git-worktree-open-elsewhere-detail = 请先关闭该窗口，然后再移除此工作树。
git-worktree-switch-to-window = 切换到该窗口
git-worktree-close-window-first = 请先关闭工作树「{ $name }」所在的窗口，然后再移除此工作树
git-worktree-remove-confirm = 要移除工作树「{ $name }」吗？
git-worktree-remove-confirm-detail = 工作树目录将移到{ common-trash }，可从{ common-trash }中恢复。
git-worktree-remove-unreachable = 分离 HEAD 上有 { $count } 个提交不属于任何分支，移除后将无法恢复。如需保留，请先在该工作树中为其新建分支。
git-worktree-modified-files = { $count } 个已修改的文件
git-worktree-untracked-files = { $count } 个未跟踪的文件
git-worktree-files-separator = 、
git-worktree-remove-uncommitted = 此工作树中有{ $files }尚未提交。
git-worktree-remove-button = 移除
git-worktree-remove-delete-branch = 同时删除分支「{ $branch }」
git-worktree-close-dialog-first = 请先关闭当前对话框，然后再移除工作树
git-worktree-trash-failed = 无法移除工作树「{ $name }」：{ $error }
git-worktree-remove-record-failed = 目录已移到{ common-trash }，但未能删除 Git 中的工作树记录（{ $error }），请稍后使用「清理失效的工作树」
git-worktree-branch-deleted = 分支「{ $branch }」已删除
git-worktree-branch-delete-failed = 目录已移到{ common-trash }，但未能删除分支「{ $branch }」：{ $error }
git-op-checkout-new-worktree = 在新工作树中签出「{ $name }」
git-op-checkout-commit-new-worktree = 在新工作树中签出 { $commit }
git-op-prune-worktrees = 清理失效的工作树
git-op-check-worktree = 检查工作树「{ $name }」
git-op-remove-worktree = 移除工作树「{ $name }」

## 日志的详情栏

git-log-no-selection = 选择一个提交
git-log-no-selection-detail = 在左侧选择提交，查看提交说明和更改的文件
git-log-details-commit = 提交
git-log-details-parents = { $count ->
   *[other] 父提交
}
git-log-details-loading-files = 正在读取更改的文件…
git-compare-title = 比较两个版本
git-compare-old = 旧
git-compare-new = 新
git-compare-no-differences = 两个版本没有差异
git-compare-comparing = 正在比较…
git-log-hint-select-file = 选择文件
git-log-hint-open-diff = 打开差异
git-log-hint-return = 返回
git-log-hint-file-list = 文件列表
git-log-hint-exit-compare = 退出比较
git-log-hint-click = 单击
git-log-hint-view-commit = 查看单个提交
git-log-hint-double-click = 双击
git-log-hint-cmd-click = ⌘ 单击
git-log-hint-compare = 比较
git-log-hint-go-back = 后退

## 提交时间

git-time-just-now = 刚刚
git-time-minutes-ago = { $count } 分钟前
git-time-today = 今天 { $time }
git-time-yesterday = 昨天 { $time }
git-time-date = { $month }月{ $day }日
git-time-date-year = { $year }年{ $month }月{ $day }日

## 提交和回滚

git-commit-message-required = 请先填写提交信息
git-commit-unresolved-conflicts = { $count } 个文件还有未解决的冲突，解决后再提交
git-commit-nothing-checked = 未勾选要提交的文件
git-commit-done = 已提交 { $oid }：{ $subject }
git-commit-failed = 无法提交：{ $error }
git-commit-done-in = 已在「{ $name }」中提交 { $oid }
git-commit-failed-in = 无法在「{ $name }」中提交：{ $error }
git-undo-commit-confirm = 要撤销上一次提交吗？
git-undo-commit-confirm-detail = 「{ $subject }」的更改会回到暂存区，提交信息会填回输入框。
git-undo-commit-confirm-button = 撤销提交
git-force-push-confirm = 要强制推送「{ $branch }」吗？
git-force-push-confirm-detail = 已推送的提交在本地被改写。上游的 { $count } 个提交将被本地版本替换；如果上游在此期间有新提交，推送将被拒绝。
git-force-push-confirm-button = 强制推送
git-commit-push-confirm = 要提交并推送吗？
git-commit-push-confirm-scope = 将 { $count } 个文件提交到 { $branch }，然后推送到上游。
git-commit-push-confirm-detail = 提交后将推送到上游。
git-commit-push-confirm-button = 提交并推送
git-undo-commit-done = 已撤销提交 { $oid }，更改保留在暂存区中
git-undo-commit-done-in = 已在「{ $name }」中撤销提交 { $oid }
git-undo-commit-failed = 无法撤销提交：{ $error }
git-commit-partial-not-utf8 = 「{ $name }」不是 UTF-8 文本，无法只提交部分更改
git-abort-confirm = 要中止{ $operation }吗？
git-abort-confirm-detail = 工作区将恢复到开始之前的状态，已解决的冲突也会丢弃。
git-abort-confirm-button = 中止{ $operation }
git-rollback-nothing-checked = 未勾选要回滚的文件
git-rollback-confirm-one = 要回滚「{ $name }」的更改吗？
git-rollback-confirm-many = 要回滚 { $count } 个文件的更改吗？
git-rollback-confirm-detail = 已跟踪的文件将恢复为 HEAD 中的内容；未跟踪的文件将移到{ common-trash }，可从{ common-trash }中恢复。
git-rollback-confirm-button = 回滚
git-rollback-file-failed = 无法回滚「{ $name }」：{ $error }
git-rollback-done = 已回滚 { $count } 个文件
git-rollback-failed = 无法回滚：{ $error }
git-rollback-done-in = 已在「{ $name }」中回滚 { $count } 个文件
git-rollback-failed-in = 无法在「{ $name }」中回滚：{ $error }
git-read-file-failed = 无法读取「{ $name }」：{ $error }

## 仓库状态和文件的差异

git-unusable = 无法使用 Git：{ $error }
git-not-a-repository = 当前目录不是 Git 仓库
git-no-file-in-tab = 当前标签页没有对应的文件
git-diff-title = { $name } · 差异
git-diff-submodule = 「{ $name }」是子模块，不显示差异
git-diff-head-unreadable = 无法读取「{ $name }」在 HEAD 中的版本：{ $error }
git-diff-too-large = 「{ $name }」过大，不显示差异
git-diff-binary = 「{ $name }」是二进制文件，不显示差异
git-diff-identical = 「{ $name }」与 HEAD 相同，没有更改
git-diff-side-worktree-deleted = 工作区 · 已删除
git-diff-side-head-missing = HEAD · 文件不存在

## 信任仓库的配置

git-trust-confirm = 是否信任仓库「{ $name }」？
git-trust-confirm-detail = 此仓库配置了以下程序，会在提交、推送等 Git 操作时自动运行。请仅在确认仓库来源可靠时选择「信任」。不信任时，winc 仍会显示文件的更改状态，但提交、切换分支等会运行这些程序的操作将不可用。
git-trust-more = ……共 { $count } 项
git-trust-deny = 不信任
git-trust-allow = 信任

## 提交视图

git-commit-view-problem = { $problem }。可在右上角「⋯」中刷新重试
git-commit-view-loading = 正在读取仓库状态…
git-commit-view-empty = 没有更改
git-commit-view-empty-detail = 工作区与 HEAD 一致
git-operation-banner = 正在{ $operation }
git-operation-banner-resolved = 正在{ $operation } · 冲突已解决
git-operation-banner-conflicts = 正在{ $operation } · { $count } 个文件有冲突
git-operation-abort = 中止
git-commit-message-placeholder = 提交信息（{ $key } 提交）
git-commit-message-amend-placeholder = 修改后的提交信息
git-branch-picker-placeholder = 搜索或新建分支
git-branch-picker-name = 新分支名称
git-branch-picker-from = 基于 { $branch } 创建
git-branch-picker-from-carry = 基于 { $branch } 创建，{ $count } 个文件的未提交更改将保留在工作区
git-branch-picker-new = 新建分支
git-branch-picker-new-worktree = 新建工作树
git-branch-picker-create-row = 新建分支「{ $name }」
git-branch-picker-current = 当前
git-picker-back = 返回
git-location-picker-placeholder = 搜索仓库和工作树
git-location-picker-empty = 没有匹配的仓库或工作树
git-location-main-worktree = 主工作树
git-location-only-one = 当前目录只有一个仓库和工作树
git-location-new-window = 在新窗口中打开
git-location-remove = 移除工作树…
git-location-prune = 清理失效的工作树
git-location-new-worktree-in = 在「{ $repository }」中新建工作树
git-location-count = { $count } 个工作树
git-tip-other-worktrees = 其他 { $count } 个工作树有未提交的更改
git-tip-other-locations = 其他 { $count } 个仓库或工作树有未提交的更改
git-tip-fetch = 获取远程更新
git-tip-in-sync = 获取远程更新（已与 { $upstream } 同步）
git-tip-push = 推送 { $count } 个提交到 { $upstream }
git-tip-pull = 从 { $upstream } 拉取 { $count } 个提交
git-tip-both = 与 { $upstream } 同步：拉取 { $behind } 个提交，推送 { $ahead } 个提交
git-tip-publish = 发布分支：推送到远程仓库并设置上游
git-switch-dirty = 签出「{ $branch }」前如何处理未提交的更改？
git-switch-dirty-detail = 「{ $current }」有 { $count } 个文件存在未提交的更改。迁移后更改保留在工作区；储藏后可随时恢复。
git-switch-carry = 迁移更改
git-switch-stash = 储藏并签出
git-switch-failed-restored = { $error }（未提交的更改已恢复）
git-switch-failed-stashed = { $error }（未提交的更改还在储藏里，可以从储藏列表恢复）
git-switch-stash-message = 切换分支前 { $branch } 上的更改
git-commit-button-nothing = 没有要提交的更改
git-branch-picker-empty = 没有匹配的分支
git-branch-picker-recent = 最近使用
git-branch-picker-other = 其他本地分支
git-commit-button = 提交
git-commit-button-files = 提交 { $count } 个文件
git-commit-button-amend = 修改提交
git-commit-button-committing = 正在提交…
git-commit-button-committed = 已提交 { $oid }
git-commit-button-continue-rebase = 继续变基
git-commit-button-finish-merge = 完成合并
git-commit-button-finish-cherry-pick = 完成拣选
git-commit-button-finish-revert = 完成还原

## 日志

git-log-filter-placeholder = 按说明或提交号筛选
git-log-filter-placeholder-key = 按说明或提交号筛选（{ $key }）
git-log-filter-all = 全部
git-log-filter-branch = 分支
git-log-filter-author = 作者
git-log-filter-file = 文件 { $path }
git-log-clear-filter = 清除筛选
git-log-refreshing = 正在刷新…
git-log-loading = 正在读取…
git-log-commit-count = { $count }{ $more } 个提交
git-log-uncommitted = 未提交的更改
git-log-uncommitted-files = { $count } 个文件
git-log-error = 无法读取提交
git-log-loading-commits = 正在读取提交…
git-log-no-matches = 没有符合筛选条件的提交
git-log-no-matches-detail = 请尝试更换分支、作者或关键词
git-log-empty = 暂无提交
git-log-empty-detail = 提交后将显示在此处

## 文件被其他程序修改时的冲突

git-disk-conflict-compare = 比较
git-disk-conflict-take-disk = 使用磁盘版本
git-disk-conflict-keep-mine = 保留当前修改
git-disk-conflict-kept-mine = 已保留当前修改，下次保存时将覆盖磁盘上的版本
git-disk-conflict-side-disk = 磁盘上的版本
git-disk-conflict-side-unsaved = 未保存的修改
git-disk-conflict-identical = 「{ $name }」与磁盘上的版本内容相同，冲突已解除
git-disk-conflict-compare-title = 比较：{ $name }
git-disk-conflict-took-disk = 「{ $name }」已改用磁盘上的版本，按 ⌘Z 可恢复未保存的修改
git-disk-conflict-reloaded = 已重新加载「{ $name }」，未保存的修改已放弃
git-disk-conflict-overwrite = 覆盖
git-disk-conflict-modified = 「{ $name }」已被其他程序修改
git-disk-conflict-modified-detail = 保存将以本窗口中的内容覆盖磁盘上的版本，建议先比较两者的差异。
git-disk-conflict-choose-first = 「{ $name }」已被其他程序修改，请先在右上角的冲突提示中选择要保留的版本

## 新建分支、新建工作树的对话框

git-form-base-current-branch = { $name }（当前分支）
git-form-base-current-commit = 当前提交
git-form-base-commit = 提交 { $commit }
git-form-base-existing-branch = 现有分支
git-form-base-current = 当前分支
git-form-name-leading-dash = 分支名称不能以「-」开头
git-form-name-exists = 已存在同名的分支
git-form-current-branch-held = 当前分支无法在其他工作树中签出
git-form-stale-worktree = 已被失效的工作树签出，请先清理
git-form-create = 创建
git-form-open = 打开
git-op-new-worktree = 新建工作树「{ $name }」

## 日志中提交的操作

git-op-checkout-commit = 签出 { $commit }
git-op-cherry-pick = 拣选 { $commit }
git-op-revert = 还原 { $commit }
git-op-reset = 将「{ $branch }」重置到 { $commit }
git-op-reset-hard = 将「{ $branch }」硬重置到 { $commit }
git-op-stash-drop = 删除储藏「{ $name }」
git-log-copied-oid = 已复制提交号 { $commit }
git-log-copied-short-oid = 已复制短提交号 { $commit }
git-log-copied-message = 已复制提交说明
git-log-detach-confirm = 要签出提交 { $commit } 吗？
git-log-detach-confirm-detail = 签出后将处于分离 HEAD 状态，不在任何分支上；在此期间的提交需要新建分支才能保留。
git-log-detach-confirm-button = 签出
git-log-reset-hard-confirm = 要将「{ $branch }」硬重置到 { $commit } 吗？
git-log-reset-hard-confirm-detail = 工作区和暂存区中所有未提交的更改都将丢弃，「{ $branch }」上此提交之后的提交也将不再属于该分支。此操作无法撤销。
git-log-reset-hard-confirm-button = 硬重置

## 日志标签和文件历史

git-log-title = Git 日志
git-history-title = { $name } · 历史
git-log-target-not-loaded = 目标提交尚未加载，请向下滚动列表后重试
git-log-parent-not-loaded = 父提交尚未加载，请向下滚动列表后重试
git-next-file = 已切换到下一个文件：{ $name }（{ $index }/{ $total }）
git-previous-file = 已切换到上一个文件：{ $name }（{ $index }/{ $total }）
git-press-again-next-file = 已是最后一处更改。再次执行可切换到下一个文件：{ $name }
git-press-again-previous-file = 已是第一处更改。再次执行可切换到上一个文件：{ $name }
git-diff-side-commit-missing = { $commit } · 文件不存在
git-diff-side-before-root = 空 · 根提交之前

## 解决合并冲突

git-merge-side-current = 当前
git-merge-side-incoming = 传入
git-merge-side-base = 共同祖先
git-merge-compare = 比较
git-merge-previous = 上一处
git-merge-next = 下一处
git-merge-mark-resolved = 标记为已解决
git-merge-accept-all-current = 全部采用当前
git-merge-accept-all-incoming = 全部采用传入
git-merge-card-conflicts = { $count } 处冲突
git-merge-card-resolved = 冲突均已解决
git-merge-compare-title = { $name } · 冲突 { $index }
git-merge-all-resolved = 所有冲突均已解决，保存后将标记为已解决
git-merge-picked-current = 已采用当前，还剩 { $count } 处冲突
git-merge-picked-incoming = 已采用传入，还剩 { $count } 处冲突
git-merge-picked-both = 已保留两者，还剩 { $count } 处冲突
git-merge-picked-all-current = { $count } 处冲突均已采用当前
git-merge-picked-all-incoming = { $count } 处冲突均已采用传入
git-merge-picked-all-both = { $count } 处冲突均已保留两者
git-merge-no-next = 没有下一处冲突
git-merge-no-previous = 没有上一处冲突
git-merge-unresolved-left = 仍有未解决的冲突
git-op-mark-resolved = 标记「{ $name }」为已解决

## 编辑器中的行内更改

git-change-line = 第 { $line } 行
git-change-lines = 第 { $start }–{ $end } 行
git-change-added = 新增 · { $lines }
git-change-modified = 修改 · { $lines }
git-change-deleted-at-start = 删除 { $count } 行 · 文件开头
git-change-deleted-after = 删除 { $count } 行 · 第 { $line } 行之后
git-change-reverted = 已回滚更改
git-change-no-next = 没有下一处更改
git-no-next-file = 没有下一个文件
git-no-previous-file = 没有上一个文件
git-change-no-previous = 没有上一处更改
git-change-none-at-caret = 光标所在行没有更改

## 差异视图

git-diff-side-worktree = 工作区
git-diff-side-worktree-unsaved = 工作区 · 未保存
git-diff-identical-sides = 两侧内容相同
git-diff-none-at-caret = 光标处没有更改
git-diff-revert-failed = 无法回滚更改：{ $error }

## 更改的弹出窗口

git-change-popup-added = 新增 { $count } 行
git-change-popup-deleted = 删除 { $count } 行
git-change-popup-modified = 修改 { $count } 行
git-change-popup-not-in-head = HEAD 中不存在
git-change-popup-more-lines = 另有 { $count } 行未显示

## 提交视图的文件列表

git-commit-group-changes = 更改
git-commit-group-conflicts = 冲突
git-commit-group-untracked = 未跟踪的文件
git-commit-open-deleted = 文件已删除，无法打开
git-commit-blocker-conflicts = { $count } 个文件还有未解决的冲突
git-log-files-changed = 更改了 { $count } 个文件
git-log-file-binary = 二进制

## 逐行追溯

git-blame-not-in-repository = 当前文件不在 Git 仓库中，无法逐行追溯
git-blame-failed = 无法逐行追溯：{ $error }
git-blame-uncommitted = 尚未提交的更改
git-blame-line-uncommitted = 此行尚未提交
