## 读取配置文件时发现的问题。

config-read-failed = 无法读取配置文件（{ $error }），已使用默认设置
config-invalid-json = 配置文件不是有效的 JSON，已使用默认设置
config-root-not-object = 配置文件的顶层应为对象，已使用默认设置
config-unknown-key = 无法识别 { $key }，已忽略
config-project-read-failed = 无法读取（{ $error }），已忽略
config-project-invalid-json = 不是有效的 JSON 对象，已忽略
config-project-unsupported-key = 项目设置仅支持 format，已忽略 { $key }

## 运行配置（.winc/run.json）。

config-run-read-failed = 无法读取（{ $error }），已忽略
config-run-invalid-json = 文件格式无效，应为包含 configurations 数组的 JSON 对象，已忽略
config-run-unsupported-key = 不支持字段 { $key }，已忽略
config-run-invalid-entry = 第 { $index } 项无效，已忽略：name 和 command 必须是非空字符串，args 必须是字符串数组，env 必须是值为字符串的对象
config-run-duplicate-name = 配置名「{ $name }」重复，已忽略后面的一项
config-run-template-note = 每项需要 name 和 command；args、cwd、env 为可选项。

## 各项设置的值不符合要求时。

config-theme-invalid = { $name } 的值只能是 auto、dark 或 light，已忽略
config-render-whitespace-invalid = { $name } 的值只能是 none、selection、trailing 或 all，已忽略
config-vim-clipboard-invalid = { $name } 的值只能是 register、yank 或 always，已忽略
config-vim-handle-keys-invalid = { $name } 应为对象，各项的值只能是 vim 或 app，已忽略
config-vim-leader-invalid = { $name } 应为一个按键的记法，已忽略
config-vim-keys-invalid = { $name } 应为映射的数组，各项含 mode（normal、visual、insert 或 operator）、keys 与 to，已忽略
config-background-fit-invalid = { $name } 的值只能是 fill、fit、stretch 或 tile，已忽略
config-language-invalid = { $name } 的值只能是 auto、zh-CN 或 en-US，已忽略
config-not-integer = { $name } 应为整数（当前为 { $value }），已忽略
config-not-string = { $name } 应为字符串，已忽略
config-terminal-position-invalid = { $name } 应为 "bottom" 或 "right"，已忽略
config-title-bar-style-invalid = { $name } 应为 "custom" 或 "native"，已忽略
config-option-as-meta-invalid = { $name } 应为 "left"、"right"、"both" 或 "none"，已忽略
config-not-array = { $name } 应为数组，已忽略
config-item-not-non-empty-string = { $name } 中的项应为非空字符串，已忽略
config-item-not-string = { $name } 中的项应为字符串，已忽略
config-not-object = { $name } 应为对象，已忽略
config-duplicate-language = { $name } 中「{ $language }」与另一项指向同一种语言，已使用「{ $language }」
config-unknown-language = { $name } 中有无法识别的语言「{ $language }」，已忽略
config-empty = { $name } 为空，已忽略
config-not-bool = { $name } 应为 true 或 false，已忽略
config-not-number = { $name } 应为数字，已忽略
config-out-of-range = { $setting } 超出{ $label }范围（{ $range }），已忽略
config-label-font-size = 字号
config-label-font-weight = 字重
config-label-background-opacity = 背景透明度
config-label-zoom = 界面缩放
config-label-tab-width = 制表符长度
config-label-vim-scroll-off = 光标与窗口边缘之间保留的行数
config-label-vim-timeout-len = 映射等待后续按键的毫秒数
config-label-vim-yank-highlight = 复制后高亮的毫秒数
config-label-repository-scan-depth = 仓库扫描深度
config-label-lines = 行数
config-label-contrast = 对比度
