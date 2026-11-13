## 任务名称。

format-task-format = 格式化
format-task-fix = 修复
format-cancelled = 已取消

## 设置中的格式化工具行。

format-tool-custom = 自定义
format-tool-from-project = 由项目设置（.winc/config.json）指定
format-tool-auto = 自动
format-tool-auto-reason = 自动 · 根据 { $reason }
format-tool-auto-installed = 自动 · 已安装
format-tool-auto-with-reason = { $name }（{ $reason }）
format-tool-unset = 未设置
format-tool-fix-unset = 项目未配置修复工具，请选择要使用的工具
format-tool-no-auto = 不自动选择工具，请选择要使用的工具
format-tool-no-preset = 无预设的工具，请选择「自定义命令…」
format-tool-searching = 正在查找已安装的工具…
format-tool-not-installed = 未安装
format-tool-not-installed-names = 未安装 { $names }
format-tool-list-separator = { " 或 " }
format-tool-reads-disk = 该命令读取磁盘上的文件，不包含未保存的修改
format-tool-not-found = 未找到「{ $tool }」
format-install-link = 安装方法…
format-tool-custom-command = 自定义命令…
format-tool-remove = 移除

## 运行前的确认。

format-approval-on-save-what = 项目设置开启的「保存时{ $task ->
        [fix] 修复
       *[other] 格式化
    }」
format-approval-on-save-detail = 此项目的 .winc/config.json 要求每次保存前先{ $task ->
        [fix] 修复
       *[other] 格式化
    }。{ $task ->
        [fix] 修复
       *[other] 格式化
    }工具将读取并执行项目中的配置，请仅在信任此项目时允许。
format-approval-project-command = 项目设置中的{ $task ->
        [fix] 修复
       *[other] 格式化
    }命令
format-approval-project-command-detail = 命令来自 .winc/config.json：{ $command }
format-approval-project-tool = 项目中的「{ $tool }」
format-approval-project-tool-detail = { $path } 由此项目提供。请仅在信任此项目时运行。
format-approval-allow-question = 允许{ $what }吗？
format-approval-run-question = 允许运行{ $what }吗？
format-approval-allow = 允许
format-approval-run = 运行
format-approval-use-system = 使用系统中安装的版本
format-approval-deny = 不允许
format-approval-declined = 已选择不运行{ $what }
format-approval-needed = 需要先确认是否允许运行{ $what }

## 格式化的结果。

format-unsaved-no-language = 文件尚未保存，无法确定语言，请在状态栏中选择语言
format-unsaved-reads-disk = 文件尚未保存，{ $language } 的{ $task ->
        [fix] 修复
       *[other] 格式化
    }命令需要读取磁盘上的文件
format-save-note = 使用 { $tool } { $task ->
        [fix] 修复
       *[other] 格式化
    }了 { $count } 行
format-save-step-failed = 未{ $task ->
        [fix] 修复
       *[other] 格式化
    }：{ $error }
format-saved-with-notes = 已保存 { $name }，{ $notes }
format-note-separator = ，
format-clean = 格式正确，无需更改
format-clean-selection = 所选内容格式正确，无需更改
format-clean-named = 「{ $name }」格式正确，无需更改
format-fix-clean = 无需修复
format-fix-clean-named = 「{ $name }」无需修复
format-modified-during = 文件在{ $task ->
        [fix] 修复
       *[other] 格式化
    }期间被修改，未应用结果
format-modified-during-named = 「{ $name }」在{ $task ->
        [fix] 修复
       *[other] 格式化
    }期间被修改，未应用结果
format-applied = 已使用 { $tool } { $task ->
        [fix] 修复
       *[other] 格式化
    }，更改了 { $count } 行
format-applied-selection = 已使用 { $tool } { $task ->
        [fix] 修复
       *[other] 格式化
    }所选内容，更改了 { $count } 行
format-applied-named = 已使用 { $tool } { $task ->
        [fix] 修复
       *[other] 格式化
    }「{ $name }」，更改了 { $count } 行
format-failed = 无法{ $task ->
        [fix] 修复
       *[other] 格式化
    }：{ $error }
format-failed-named = 无法{ $task ->
        [fix] 修复
       *[other] 格式化
    }「{ $name }」：{ $error }

## 运行工具时的错误。

format-unknown-language = 无法识别文件的语言
format-no-fixer-configured = 项目未配置 { $language } 的修复工具（{ $names }），可在设置中指定
format-no-auto-tool = { $language } 无可自动使用的{ $task ->
        [fix] 修复
       *[other] 格式化
    }工具，请在设置中指定命令
format-command-empty = 命令为空
format-command-unclosed-single-quote = 命令中的单引号未闭合
format-command-unclosed-double-quote = 命令中的双引号未闭合
format-no-formatter-installed = 未安装 { $language } 格式化工具（{ $names }）
format-no-range = 「{ $tool }」不支持格式化所选内容
format-preset-tool-not-found = 项目使用 { $preset }（{ $reason }），但未找到「{ $tool }」
format-no-permission = 「{ $tool }」无执行权限
format-spawn-failed = 「{ $tool }」启动失败（{ $error }）
format-timeout = 「{ $tool }」超过 { $count } 秒未完成。新安装的工具首次运行可能较慢，请重试
format-output-too-large = 「{ $tool }」的输出超过 { $size } MB
format-exit-code = 退出码 { $code }
format-killed-by-signal = 被信号终止
format-transcript-note = （工作目录：文件所在目录，{ $status }）
format-tool-error = 「{ $tool }」运行失败（退出码 { $code }）
format-tool-exited = 「{ $tool }」意外退出
format-no-output = 「{ $tool }」未返回任何输出
format-output-not-utf8 = 「{ $tool }」的输出不是 UTF-8 文本
format-eslint-unreadable = 无法读取「{ $tool }」的结果
format-eslint-parse-error = 无法解析
format-eslint-fatal = { $message }（{ $line }:{ $column }）
format-error-position = （{ $line }:{ $column }）
format-file-too-large = 文件过大

## 安装说明。

format-install-title = 安装格式化工具
format-install-not-found-one = 未找到「{ $tool }」。
format-install-not-found-many = 未找到格式化工具。安装以下任一工具后，Winc 将使用第一个已安装的工具：{ $tools }。
format-install-quoted = 「{ $name }」
format-install-list-separator = 、
format-install-intro =
    Winc 不内置格式化和修复工具，使用系统或项目中已安装的工具。
    安装后重试，无需重启 Winc。
format-install-run-any = 在终端中运行以下任一命令（需先安装对应的包管理器）：
format-install-run-here = 在终端中运行（本机可用）：
format-install-other = 其他方式：
format-install-docs = 文档：{ $url }
format-install-project-declared = 项目的 package.json 已声明此依赖，安装项目依赖即可：
format-install-add-to-project = 添加为项目的开发依赖（与团队使用同一版本）：
format-install-note-rustfmt = rustfmt 随 Rust 工具链提供；缺失时可使用 rustup 单独安装。
format-install-note-gofmt = gofmt 随 Go 提供。
format-install-note-eslint = ESLint 通常安装在项目中；此命令将安装 ESLint 并创建配置文件。
format-install-note-clang-format = macOS 上的 Xcode 和命令行开发者工具已包含 clang-format。
format-install-note-swift-format = Xcode 16 及以上版本和命令行开发者工具已包含 swift-format。
format-install-note-xmllint = macOS 已预装 xmllint。
