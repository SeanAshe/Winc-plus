## 设置卡片。

settings-language = 语言
settings-language-auto = 跟随系统
settings-title = 设置

## 分类与分组。

settings-page-appearance = 外观
settings-page-editor = 编辑器
settings-page-format = 格式化
settings-page-modules = 模块
settings-page-keys = 快捷键
settings-page-about = 关于
settings-section-format-tools = 格式化工具
settings-section-fix-tools = 修复工具

## 设置项的名称。

settings-theme = 模式
settings-light-theme = 浅色主题
settings-dark-theme = 深色主题
settings-ui-font = 界面字体
settings-ui-size = 界面字号
settings-editor-font = 编辑器字体
settings-editor-size = 编辑器字号
settings-tab-width = 制表符长度
settings-auto-save = 自动保存
settings-hot-exit = 热退出
settings-restore-folder = 启动时恢复上次打开的目录
settings-sticky-scroll = 粘滞滚动
settings-word-wrap = 自动换行
settings-compact-folders = 紧凑文件夹
settings-minimap = 缩略图
settings-remote-images = Markdown 预览显示网络图片
settings-check-updates = 自动检查更新
settings-system-title-bar = 使用系统标题栏
settings-format-on-save = 保存时格式化
settings-fix-on-save = 保存时修复问题
settings-skip-on-save = 排除的语言
settings-add-language = 添加语言…
settings-terminal-module = 终端
settings-key-search = 搜索快捷键

## 设置项的说明。

settings-tab-width-desc = 一个制表符显示的宽度（空格数）；空格缩进按文件内容判断
settings-auto-save-desc = 切换标签页或切换到其他应用时自动保存修改
settings-hot-exit-desc = 退出时不逐个询问是否保存，下次启动时恢复未保存的修改
settings-restore-folder-desc = 退出时记录打开的目录和标签页，下次启动时恢复
settings-sticky-scroll-desc = 滚动时在顶部固定显示当前所在的函数、类和标题
settings-word-wrap-desc = 长行按编辑区宽度换行显示，不修改文件内容
settings-compact-folders-desc = 把文件树中连续的单子目录合并显示为一行
settings-minimap-desc = 在编辑区右侧显示代码的缩略图，点击或拖动可以滚动
settings-remote-images-desc = 从网络加载 README 中的徽章和截图；关闭后显示替代文字
settings-check-updates-desc = 启动时及之后每天检查一次，发现新版本时在状态栏中提示
settings-system-title-bar-desc = 由桌面环境绘制标题栏和窗口边框
settings-version = 版本
settings-updates = 软件更新
settings-check-now = 检查更新
settings-updates-off = 此版本不支持应用内更新
settings-updates-idle = 尚未检查更新
settings-updates-checking = 正在检查更新…
settings-updates-latest = winc 已是最新版本
settings-updates-found = 有可用的新版本：{ $version }
settings-updates-failed = 无法检查更新，请稍后重试
settings-updates-downloading = 正在下载 { $version }…
settings-updates-ready = { $platform ->
    [windows] { $version } 已下载，重新启动或退出 winc 时安装
   *[other] { $version } 已安装，重新启动 winc 后生效
}
settings-updates-install-failed = 无法安装更新：{ $error }
settings-format-on-save-desc = 保存文件前使用下方的工具进行格式化
settings-fix-on-save-desc = 保存文件前自动修复 lint 工具可修复的问题
settings-skip-on-save-desc = 保存以下语言的文件时不执行修复和格式化，手动执行不受影响
settings-git-module-desc = 在 Git 仓库中显示文件状态和当前分支
settings-terminal-module-desc = 编辑区下方的终端面板（⌃`），打开时启动一个辅助进程

## 设置项的值和提示。

settings-theme-auto = 跟随系统
settings-theme-light = 浅色
settings-theme-dark = 深色
settings-skip-on-save-none = 无
settings-list-separator = 、
settings-font-default = 系统默认
settings-font-others = 其他字体
settings-command-placeholder = 输入命令，{ "{file}" } 表示文件路径
settings-command-edit-hint = 按 ↩ 保存，按 Esc 取消
settings-font-not-installed = 未安装字体「{ $name }」
settings-font-file-not-found = 未找到字体文件「{ $name }」
settings-font-unsupported = 不支持字体「{ $name }」的字形格式
settings-theme-used-when-dark = 系统使用深色外观时采用此主题
settings-theme-used-when-light = 系统使用浅色外观时采用此主题

## 主题名称。

settings-theme-name-winc-day = Winc 晴昼
settings-theme-name-winc-paper = Winc 宣纸
settings-theme-name-sky = 晴空
settings-theme-name-paper = 素纸
settings-theme-name-vermilion = 朱砂
settings-theme-name-mist = 晨雾
settings-theme-name-winc-night = Winc 夜航
settings-theme-name-winc-slate = Winc 远山
settings-theme-name-ink = 墨蓝
settings-theme-name-ember = 炭火
settings-theme-name-midnight = 深夜
settings-theme-name-dusk = 暮紫

## 快捷键页。

settings-keys-recording = 请按下要查找的快捷键，按 Esc 停止
settings-keys-search-record = 输入命令名查找，按 { $key } 录制按键
settings-keys-search = 输入命令名查找
settings-keys-no-command = 没有命令使用此快捷键
settings-keys-no-match = 没有匹配的命令

## 字号与配置文件。

settings-zoom-limit = 编辑区字号已为 { $size }，无法继续调整
settings-zoom = 编辑区字号 { $size }
settings-config-backed-up = 原配置文件无法解析，已另存为 { $path } 后写入新设置
settings-config-comments-backed-up = 写入设置时无法保留配置文件里的注释，原文件已另存为 { $path }
settings-config-write-failed = 无法写入配置文件：{ $error }
