## 多个领域共用的文案：通用按钮和选项。

common-cancel = 取消
common-dont-ask = 不再询问
common-open = 打开
common-save = 保存
common-save-as-label = 存储为：
common-list-separator = 、
common-reveal-in-finder = 在访达中显示
common-reveal-in-folder = { $platform ->
    [windows] 在文件资源管理器中显示
   *[other] 在目录中显示
}
common-key-enter = { $platform ->
    [macos] ↩
   *[other] Enter
}
common-close = 关闭
common-close-dialog-first = 请先关闭当前对话框

## 移到废纸篓

common-trash = { $platform ->
    [macos] 废纸篓
   *[other] 回收站
}
common-trash-confirm-folder = 要将目录「{ $name }」及其内容移到{ common-trash }吗？
common-trash-confirm-item = 要将「{ $name }」移到{ common-trash }吗？
common-trash-confirm-items = 要将这 { $count } 项移到{ common-trash }吗？
common-trash-confirm-detail = 可从{ common-trash }中恢复。
common-trash-move = 移到{ common-trash }
common-trash-moved-item = 已移到{ common-trash }：{ $name }
common-trash-moved-items = 已将 { $count } 项移到{ common-trash }
common-delete-confirm-folder = 要永久删除目录「{ $name }」及其内容吗？
common-delete-confirm-item = 要永久删除「{ $name }」吗？
common-delete-confirm-items = 要永久删除这 { $count } 项吗？
common-delete-confirm-detail = 远端主机上的文件不经过{ common-trash }，删除后无法恢复。
common-delete-button = 永久删除
common-deleted-item = 已删除：{ $name }
common-deleted-items = 已删除 { $count } 项
common-trash-delete-failed = 无法删除「{ $name }」：{ $error }
common-trash-invalid-path = 路径无效
common-trash-no-file-manager = 系统文件管理服务不可用
common-trash-failed = 无法移到{ common-trash }
common-trash-failed-code = 无法移到{ common-trash }（错误代码 { $code }）
common-trash-cancelled = 已取消，未移到{ common-trash }
common-trash-unsupported = 此平台不支持移到回收站

## 打开链接（终端、Markdown 预览）
common-link-scheme-not-allowed = 仅支持 http、https 和 mailto 链接

## 新建工作树、新建分支对话框

common-dialog-new-worktree = 新建工作树
common-dialog-new-branch = 新建分支
common-dialog-branch = 分支
common-dialog-name = 名称
common-dialog-base = 基于
common-dialog-location = 位置
common-dialog-worktree-placeholder = 新分支名称或现有分支
common-dialog-branch-placeholder = 新分支名称
common-dialog-branch-name = 分支名称
common-dialog-copy-ignored = 复制被忽略的文件
common-dialog-open-in-new-window = 在新窗口中打开
common-dialog-checkout-after-create = 创建后签出
common-dialog-creating = 正在创建…

## 移到「应用程序」文件夹

app-move-title = 要将 Winc 移到「应用程序」文件夹吗？
app-move-detail = Winc 当前在「{ $folder }」中直接运行，无法自动更新。移到「应用程序」文件夹后，原件将移到废纸篓，系统中只保留这一份 Winc。
app-move-detail-image = Winc 当前从磁盘映像运行，无法自动更新。移到「应用程序」文件夹后，磁盘映像将被推出，系统中只保留这一份 Winc。
app-move-button = 移到「应用程序」文件夹
app-move-stay = 不移动
app-move-failed = 无法移到「应用程序」文件夹：{ $error }
app-move-target-running = 「应用程序」文件夹中的 Winc 正在运行，请先将其退出
app-move-trash-failed = 无法将「{ $path }」移到废纸篓
app-move-copy-failed = 复制失败（{ $status }）
app-move-relaunch-failed = 无法重新启动 Winc：{ $error }
app-move-unknown-location = 无法确定 Winc 的位置
