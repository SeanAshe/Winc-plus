## 面板

terminal-dock-bottom = 停靠到底部
terminal-dock-right = 停靠到右侧
terminal-hide-panel = 隐藏终端面板
terminal-show-panel = 显示终端面板
terminal-tab-default-name = 终端
terminal-module-disabled = 终端模块未启用

## 终端组件

terminal-helper-missing = 未找到终端组件 winc-pty（开发环境下请先运行 cargo build -p winc-pty）
terminal-helper-start-failed = 无法启动终端组件：{ $error }
terminal-helper-exited = 终端组件已退出
terminal-helper-version-mismatch = 终端组件的版本与 winc 不一致，请重新安装 winc
terminal-process-exited = 进程已退出（代码 { $code }）
terminal-disconnected = 终端连接已断开
terminal-restart = ↩ 重新启动
terminal-close-hint = { $key } 关闭
terminal-restart-confirm = 要重新启动这个终端吗？
terminal-restart-confirm-detail = 这个终端的连接已断开，重新启动会结束里面仍在运行的程序。
terminal-restart-confirm-action = 重新启动

## 命令和输出

terminal-command-fallback = 命令
terminal-quoted-command = 「{ $command }」
terminal-command-failed = { $name }失败，退出码 { $code }（{ $duration }）
terminal-command-finished = { $name }已完成（{ $duration }）
terminal-command-copied = 已复制命令
terminal-no-failed-commands = 没有失败的命令
terminal-no-error-location = 最后一条失败命令的输出中未找到文件位置
terminal-error-position = 错误 { $index }/{ $total }：↩ 打开
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
terminal-quit-confirm-detail = 终端里还有程序在运行，退出 winc 会结束它们。
terminal-close-window-confirm-one = 要关闭窗口并结束「{ $name }」吗？
terminal-close-window-confirm-many = 要关闭窗口并结束 { $count } 个正在运行的程序吗？
terminal-close-window-confirm-detail = 这个窗口的终端里还有程序在运行，关闭窗口会结束它们。
terminal-switch-folder-confirm-one = 要打开另一个目录并结束「{ $name }」吗？
terminal-switch-folder-confirm-many = 要打开另一个目录并结束 { $count } 个正在运行的程序吗？
terminal-switch-folder-confirm-detail = 终端中仍有程序在运行。在此窗口中打开其他目录将关闭当前目录的终端，并结束这些程序。
terminal-switch-folder-confirm-button = 打开
terminal-close-folder-confirm-one = 要关闭目录并结束「{ $name }」吗？
terminal-close-folder-confirm-many = 要关闭目录并结束 { $count } 个正在运行的程序吗？
terminal-close-folder-confirm-detail = 终端中仍有程序在运行，关闭目录将结束这些程序。
terminal-close-confirm = 要结束「{ $name }」吗？
terminal-close-confirm-detail = 关掉这个终端标签会结束里面正在运行的程序。
terminal-paste-confirm = 要把 { $count } 行文字粘贴到终端吗？
terminal-paste-confirm-detail = 终端里的程序没有开启括号粘贴，每一行会作为单独的命令立即执行。
