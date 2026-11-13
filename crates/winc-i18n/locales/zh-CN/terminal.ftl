## 面板

terminal-dock-bottom = 停靠到底部
terminal-dock-right = 停靠到右侧
terminal-hide-panel = 隐藏终端面板
terminal-show-panel = 显示终端面板
terminal-tab-default-name = 终端
terminal-module-disabled = 终端模块未启用

## 终端组件

terminal-helper-missing = 未找到终端组件，请重新安装 Winc
terminal-helper-start-failed = 无法启动终端组件：{ $error }
terminal-helper-exited = 终端组件已退出
terminal-helper-version-mismatch = 终端组件的版本与 Winc 不一致，请重新安装 Winc
terminal-process-exited = 进程已退出（退出码 { $code }）
terminal-disconnected = 终端连接已断开
terminal-restart = 重新启动
terminal-restart-confirm = 要重新启动此终端吗？
terminal-restart-confirm-detail = 此终端的连接已断开，重新启动将结束其中仍在运行的程序。
terminal-restart-confirm-action = 重新启动

## 命令和输出

terminal-command-fallback = 命令
terminal-quoted-command = 「{ $command }」
terminal-command-failed = { $name }失败，退出码 { $code }（{ $duration }）
terminal-command-finished = { $name }已完成（{ $duration }）
terminal-command-copied = 已复制命令
terminal-no-failed-commands = 无失败的命令
terminal-no-error-location = 最后一条失败命令的输出中未找到文件位置
terminal-error-position = 错误 { $index }/{ $total }：{ common-key-enter } 打开
terminal-folded-lines-unit = { $count ->
       *[other] 行
    }
terminal-selection-copied = 已复制所选内容
terminal-link-open-failed = 无法打开链接：{ $error }
terminal-notification = { $title }：{ $body }

## 查找

terminal-find-placeholder = 在终端中查找
terminal-find-searching = 正在查找…
terminal-find-no-results = 无结果

## 关闭确认

terminal-quit-confirm-one = 要退出并结束「{ $name }」吗？
terminal-quit-confirm-many = 要退出并结束 { $count } 个正在运行的程序吗？
terminal-quit-confirm-detail = 终端中仍有程序在运行，退出 Winc 将结束这些程序。
terminal-close-window-confirm-one = 要关闭窗口并结束「{ $name }」吗？
terminal-close-window-confirm-many = 要关闭窗口并结束 { $count } 个正在运行的程序吗？
terminal-close-window-confirm-detail = 此窗口的终端中仍有程序在运行，关闭窗口将结束这些程序。
terminal-switch-folder-confirm-one = 要打开另一个目录并结束「{ $name }」吗？
terminal-switch-folder-confirm-many = 要打开另一个目录并结束 { $count } 个正在运行的程序吗？
terminal-switch-folder-confirm-detail = 终端中仍有程序在运行。在此窗口中打开其他目录将关闭当前目录的终端，并结束这些程序。
terminal-switch-folder-confirm-button = 打开
terminal-close-folder-confirm-one = 要关闭目录并结束「{ $name }」吗？
terminal-close-folder-confirm-many = 要关闭目录并结束 { $count } 个正在运行的程序吗？
terminal-close-folder-confirm-detail = 终端中仍有程序在运行，关闭目录将结束这些程序。
terminal-close-confirm = 要结束「{ $name }」吗？
terminal-close-confirm-detail = 关闭此终端标签页将结束其中正在运行的程序。
terminal-paste-confirm = 要将 { $count } 行文字粘贴到终端吗？
terminal-paste-confirm-detail = 终端中的程序未开启括号粘贴，每一行将作为单独的命令立即执行。
terminal-paste-image-failed = 无法保存图片：{ $error }

## 运行配置。

terminal-shell-select = 选择 Shell
terminal-run-select = 选择运行配置
terminal-run-again = 重新运行
terminal-run-confirm = 要运行「{ $name }」吗？
terminal-run-confirm-detail = 此命令来自项目的 .winc/run.json，将以当前用户的权限执行。请仅在信任此项目时运行。
terminal-run-confirm-detected = 此命令由 Winc 根据项目文件生成，将执行项目中的代码。请仅在信任此项目时运行。
terminal-run-confirm-command = 命令：{ $command }
terminal-run-confirm-cwd = 工作目录：{ $cwd }
terminal-run-confirm-env = 环境变量：{ $variable }
terminal-run-confirm-action = 运行
terminal-run-config-problem = 运行配置（.winc/run.json）：{ $problem }
terminal-run-create-failed = 无法创建运行配置：{ $error }
terminal-run-add = 添加运行配置
terminal-run-stopped = 已停止
terminal-run-finished = 已完成
terminal-run-failed = 「{ $name }」运行失败（退出码 { $code }）
terminal-run-menu-running = 运行中 · { $command }
