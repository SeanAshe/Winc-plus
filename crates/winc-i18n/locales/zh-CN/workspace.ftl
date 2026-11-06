## 通用

workspace-open-folder-first = 请先打开一个目录
workspace-no-open-file = 没有打开的文件
workspace-list-separator = 、
workspace-quoted-name = 「{ $name }」

## 文件树和侧栏

workspace-tree-expand-limit = 目录太多，只展开了前 { $count } 个
workspace-tree-invalid-name = 名称无效
workspace-tree-name-taken = 已存在同名项
workspace-tree-cannot-rename = 无法重命名此项
workspace-tree-cannot-move = 无法移动此项
workspace-tree-move-into-itself = 无法移到其自身内部
workspace-tree-name-taken-in = 「{ $folder }」中已有同名项
workspace-tree-target-outside = 目标不在当前目录中
workspace-tree-copy-into-itself = 无法复制到其自身内部
workspace-tree-cannot-copy = 无法复制此项
workspace-tree-tool-new-file = 新建文件
workspace-tree-tool-new-folder = 新建目录
workspace-tree-tool-refresh = 刷新目录
workspace-tree-tool-collapse-all = 折叠所有目录
workspace-tree-tool-locate = 定位当前文件
workspace-sidebar-files = 文件
workspace-sidebar-search = 搜索
workspace-sidebar-commit = 提交
workspace-sidebar-no-folder = 未打开目录
workspace-tree-open-hint-key = { $key } 打开文件或目录
workspace-tree-open-hint = 打开文件或目录
workspace-tree-open-folder-hint-key = { $key } 打开目录
workspace-tree-open-folder-hint = 打开目录
workspace-tree-empty = 目录为空
workspace-tree-empty-hint = 双击此处新建文件
workspace-tree-rename-placeholder = 新名称
workspace-tree-create-placeholder = 名称，可用 / 创建子目录
workspace-tree-copy-failed = 无法复制「{ $name }」：{ $error }
workspace-tree-move-confirm-one = 要将「{ $name }」移到「{ $folder }」吗？
workspace-tree-move-confirm-many = 要将这 { $count } 项移到「{ $folder }」吗？
workspace-tree-move = 移动
workspace-tree-dont-ask = 不再询问
workspace-tree-move-failed = 无法移动「{ $name }」：{ $error }
workspace-tree-drag-items = { $count } 项
workspace-tree-drag-copy = 复制 { $name }

## 打开目录

workspace-open-folder-where = 要在哪里打开「{ $name }」？
workspace-open-folder-where-detail = 在此窗口中打开将关闭「{ $current }」及其标签页和终端。
workspace-open-folder-this-window = 此窗口
workspace-open-folder-new-window = 新窗口

## 开始页

workspace-start-settings = 设置
workspace-start-new-file = 新建文件
workspace-start-open = 打开文件或目录
workspace-start-open-file = 打开文件
workspace-start-open-folder = 打开目录
workspace-start-version = 版本 { $version }
workspace-start-section-start = 开始
workspace-start-section-recent = 最近

## 全局搜索

workspace-search-placeholder = 搜索
workspace-search-replace-placeholder = 替换
workspace-search-include = 包含
workspace-search-include-example = 例如 src, *.rs
workspace-search-exclude = 排除
workspace-search-include-ignored = 搜索被忽略的文件（.gitignore 等规则排除的文件）
workspace-search-exclude-example = 例如 *.min.js, docs
workspace-search-searching = 正在搜索…
workspace-search-summary = { $files } 个文件中有 { $total }{ $more } 个结果
workspace-search-summary-searching = { $files } 个文件中有 { $total }{ $more } 个结果，正在搜索…
workspace-search-no-results = 未找到结果
workspace-search-no-results-hint = 请尝试其他关键词，或调整匹配选项和过滤条件
workspace-search-no-results-hint-ignored = 请尝试其他关键词或调整匹配选项；被忽略的文件不在搜索范围内，可在「排除」中开启
workspace-search-not-utf8 = 非 UTF-8 编码，无法替换
workspace-search-tool-refresh = 重新搜索
workspace-search-tool-clear = 清除搜索结果
workspace-search-tool-expand-all = 全部展开
workspace-search-tool-collapse-all = 全部折叠
workspace-search-hide-filters = 收起过滤条件
workspace-search-filter-files = 过滤文件
workspace-search-row-replace-all = 全部替换
workspace-search-row-replace = 替换
workspace-search-row-dismiss = 从结果中移除
workspace-search-regex-error = 正则表达式有误：{ $error }
workspace-search-include-error = 包含规则有误：{ $error }
workspace-search-exclude-error = 排除规则有误：{ $error }
workspace-search-line-copied = 已复制此行

## 全局替换

workspace-replace-all-busy = 上一次全部替换尚未完成，请稍后重试
workspace-replace-all-searching = 搜索尚未完成，请稍后再全部替换
workspace-replace-all-too-many = 结果超过 { $max } 处，请缩小范围后再全部替换
workspace-replace-all-nothing = 没有可替换的结果
workspace-replace-all-confirm-delete = 要删除 { $files } 个文件中的 { $total } 处匹配吗？
workspace-replace-all-confirm = 要在 { $files } 个文件中将 { $total } 处替换为「{ $replacement }」吗？
workspace-replace-all-button = 替换
workspace-replace-all-warning = 未打开的文件将被直接修改，且无法撤销。
workspace-replace-all-changed = 搜索结果已变化，请重新全部替换
workspace-replace-all-done = 已在 { $files } 个文件中替换 { $count } 处
workspace-replace-all-unsaved = ，其中 { $opened } 个已打开的文件尚未保存
workspace-replace-all-stale = ；{ $stale } 处在搜索后被修改，未替换
workspace-replace-all-failed = ；{ $count } 个文件未替换（{ $error }）
workspace-replace-stale = 文件在搜索后被修改，此处已不匹配，请重新搜索
workspace-replace-failed = 无法替换：{ $error }
workspace-replace-failed-file = { $name }：{ $error }

## 查找条和跳转条

workspace-find-regex-invalid = 正则表达式有误
workspace-find-searching = 搜索中…
workspace-find-no-results = 无结果
workspace-find-count = { $count }{ $more } 个
workspace-find-placeholder = 查找
workspace-find-replace-placeholder = 替换
workspace-find-toggle-replace = 切换替换
workspace-find-match-case = 区分大小写
workspace-find-whole-word = 全字匹配
workspace-find-use-regex = 使用正则表达式
workspace-find-previous = 上一个匹配项
workspace-find-next = 下一个匹配项
workspace-find-filter-lines = 只显示匹配的行
workspace-find-close = 关闭
workspace-find-replace = 替换
workspace-find-replace-all = 全部替换
workspace-goto-title = 转到行
workspace-goto-placeholder = 行:列
workspace-goto-total = 共 { $count } 行
workspace-stash-title = 储藏更改
workspace-stash-placeholder = 说明（可选）
workspace-stash-hint = 含未跟踪的文件 · ↩ 储藏
workspace-tag-title = 新建标签
workspace-tag-placeholder = 标签名称
workspace-tag-hint = 基于 { $commit } · ↩ 创建

## 快速打开

workspace-picker-commands = { $count } 个命令
workspace-picker-symbols = { $count } 个符号
workspace-picker-files-matched = { $matched } / { $total } 个文件
workspace-picker-files = { $count } 个文件
workspace-picker-placeholder = 按文件名查找…
workspace-picker-no-folder = 请先打开一个目录；输入 > 可查找命令
workspace-picker-indexing = 正在索引文件…
workspace-picker-goto-line-column = 转到第 { $line } 行第 { $column } 列
workspace-picker-goto-line = 转到第 { $line } 行
workspace-picker-folder-empty = 此目录中没有文件
workspace-picker-no-matching-files = 未找到匹配的文件。被忽略的文件不在搜索范围内。
workspace-picker-recently-opened = 最近打开
workspace-picker-no-matching-commands = 没有匹配的命令
workspace-picker-recently-used = 最近使用
workspace-picker-symbols-too-large = 文件过大，无法列出符号
workspace-picker-no-symbols = 此文件中未找到符号
workspace-picker-no-matching-symbols = 没有匹配的符号
workspace-conflict-modified = 「{ $name }」已被其他程序修改

## 悬停提示

workspace-tip-continue-rebase = 继续变基
workspace-tip-finish-operation = 完成{ $operation }
workspace-tip-amend = 修改上一次提交
workspace-tip-commit-to = 提交到 { $branch }
workspace-tip-commit = 提交
workspace-tip-open-file = 打开文件
workspace-tip-revert-change = 回滚更改
workspace-tip-commit-staged-only = 仅提交已暂存的部分，其余更改保留在工作区中
workspace-tip-commit-menu = 更多提交方式
workspace-tip-git-log = Git 日志
workspace-tip-more-actions = 更多操作
workspace-tip-show-as-tree = 以树形显示
workspace-tip-show-as-list = 以列表显示
workspace-tip-previous-conflict = 上一处冲突
workspace-tip-next-conflict = 下一处冲突
workspace-tip-worktree-of = 「{ $name }」的工作树 · { $path }
workspace-tip-previous-change = 上一处更改
workspace-tip-next-change = 下一处更改
workspace-tip-previous-file = 上一个文件
workspace-tip-next-file = 下一个文件
workspace-tip-diff-inline = 切换为单栏视图
workspace-tip-diff-side-by-side = 切换为并排视图
workspace-tip-open-diff = 在差异视图中打开
workspace-tip-close-esc = 关闭（Esc）
workspace-tip-revert-to-left = 回滚更改（恢复为左侧内容）
workspace-tip-include-change = 提交时包含此更改
workspace-tip-expand-unchanged = 展开所有未更改的内容
workspace-tip-collapse-unchanged = 折叠未更改的内容
workspace-tip-close-tab = 关闭
workspace-tip-all-tabs = 显示所有标签页
workspace-tip-commit-changes = 提交 · { $count } 个更改
workspace-tip-log-go-to = 转到 { $commit } { $subject }
workspace-tip-log-target-not-loaded = 目标提交尚未加载

## 状态栏

workspace-status-spaces = 空格: { $size }
workspace-status-update = 有可用更新：{ $version }
workspace-status-update-downloading = 正在下载更新…
workspace-status-update-ready = 重新启动以完成更新
workspace-status-not-shown = 未显示：{ $features }
workspace-status-position-selected = { $position }（{ $count } 个字符）
workspace-status-selections = { $count } 个选区
workspace-status-selections-chars = { $count } 个选区（{ $chars } 个字符）
workspace-status-readonly = 只读
workspace-status-large-file = 大文件
workspace-status-branch-running = 正在{ $task }…
workspace-status-branch-operation = { $operation }中
workspace-status-formatting = 正在格式化…
workspace-status-fixing = 正在修复…
workspace-reopen-discard = 放弃修改并重新打开
workspace-reopen-confirm = 要放弃修改并使用其他编码重新打开吗？
workspace-reopen-confirm-detail = 所有未保存的修改都将丢失。
workspace-reopen-dirty = 有未保存的修改，请先保存或撤销后再重新打开
workspace-reopen-failed = 无法重新打开：{ $error }
workspace-reopen-lossy = 部分内容无法按 { $encoding } 解码，已以只读方式打开
workspace-indent-detected = 已从内容中检测缩进方式：{ $indent }

## 路径栏

workspace-breadcrumb-read-failed = 无法读取：{ $error }
workspace-breadcrumb-empty = 空目录
workspace-breadcrumb-more = 还有 { $count } 项未列出
workspace-key-path-copied = 已复制 { $path }
workspace-key-path-none = 光标不在 JSON、YAML 或 TOML 的键中

## 比较文件

workspace-compare-read-failed = 无法读取「{ $name }」：{ $error }
workspace-compare-too-large = 「{ $name }」过大，不显示差异
workspace-compare-binary = 「{ $name }」是二进制文件，不显示差异
workspace-compare-identical = 「{ $left }」与「{ $right }」相同

## 打开和保存文件

workspace-open-default-app-failed = 无法用默认应用打开：{ $error }
workspace-reveal-failed = 无法{ $action }：{ $error }
workspace-terminal-app-fallback = 终端
workspace-open-app-failed = 无法打开「{ $app }」：{ $error }
workspace-deleted-on-disk-key = 「{ $name }」已从磁盘上删除，未自动保存；按 { $key } 重新创建
workspace-deleted-on-disk = 「{ $name }」已从磁盘上删除，未自动保存；重新创建
workspace-save-as-dirty-elsewhere = 无法另存为「{ $name }」：该文件在另一个标签页中有未保存的修改
workspace-save-conflict = 无法保存「{ $name }」：文件已被其他程序修改，请先选择要保留的版本
workspace-save-failed = 无法保存「{ $name }」：{ $error }
workspace-saved = 已保存 { $name }
workspace-drop-to-open = 将文件或目录拖到窗口中即可打开
workspace-picker-unavailable = 系统中没有可用的选择面板。请安装 xdg-desktop-portal，或将文件、目录拖到窗口中打开
workspace-picker-failed = 无法打开选择面板：{ $error }
workspace-open-folder-failed = 无法打开目录「{ $name }」：{ $error }
workspace-recent-missing = 未找到「{ $path }」，已从最近列表中移除
workspace-reloaded = 已重新加载 { $names }
workspace-disk-conflicts = { $names } 已被其他程序修改，与未保存的修改冲突，请切换到对应的标签页选择要保留的版本

## 关闭

workspace-close-save-one = 要保存对「{ $name }」的修改吗？
workspace-close-save-detail = 如果不保存，所做的修改将会丢失。
workspace-close-save = 保存
workspace-close-more-files = ……还有 { $count } 个文件
workspace-close-save-many = 要保存对以下 { $count } 个文件的修改吗？
workspace-close-save-all = 全部保存
workspace-close-dont-save = 不保存
workspace-reopen-tab-none = 没有可重新打开的标签页

## 提示和窗口

workspace-notice-show-output = 查看输出
workspace-notice-install = 安装方法
workspace-notice-open-settings = 打开设置
workspace-format-output-title = 格式化输出
workspace-font-missing = 无法使用配置中的字体 { $fonts }，已改用默认字体（按 ⌘, 打开设置）
workspace-font-missing-item = { $key }「{ $name }」
workspace-font-unusable = 无法使用此字体，请选择其他字体
workspace-stdin-title = 标准输入
workspace-stdin-read-failed = 读取标准输入时出错：{ $error }
workspace-stdin-truncated = 只读取了标准输入的前 { $size } MB
workspace-backup-locked = 此目录已在另一个 winc 窗口中打开，本窗口未保存的修改将不会自动备份
workspace-backup-failed = 无法备份未保存的修改：{ $error }
