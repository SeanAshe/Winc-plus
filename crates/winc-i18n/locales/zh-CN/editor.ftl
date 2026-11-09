## 编辑区的占位页和欢迎页。

editor-preview-failed = 无法显示预览：{ $reason }
editor-show-source = 显示源代码
editor-reading = 正在读取「{ $name }」…
editor-open-anyway = 仍然打开
editor-open-with-default-app = 用默认应用打开
editor-retry = 重试
editor-welcome-open = 打开文件或目录
editor-welcome-open-file = 打开文件
editor-welcome-open-folder = 打开目录
editor-welcome-new-file = 新建文件
editor-welcome-go-to-file = 转到文件
editor-welcome-find-in-files = 在文件中查找
editor-welcome-toggle-sidebar = 显示或隐藏侧栏
editor-welcome-settings = 设置
editor-line-truncated = … 此行过长，仅显示前 { $count } 个字符
editor-blame-uncommitted = 未提交

## 查找、替换和多光标。

editor-no-other-match = 没有其他匹配项
editor-no-match = 没有匹配项
editor-too-many-matches = 匹配项过多，只选中了前 { $count } 处
editor-too-many-cursors = 光标过多，只保留了 { $count } 个
editor-find-large-filter = 文件过大，无法只显示匹配的行
editor-replace-none = 没有可替换的匹配项
editor-replaced = 已替换 { $count } 处
editor-find-large-multiline = 大文件不支持跨行查找
editor-find-hidden-match = 匹配项超出该行的显示范围
editor-find-hidden-match-wrap = 匹配项超出该行的显示范围，开启自动换行后可见
editor-replace-all-pending = 查找尚未完成，请稍后再全部替换
editor-replace-all-too-many = 匹配超过 10000 处，无法全部替换
editor-replace-all-gave-up = 正则在部分行上回溯过多，查找结果不完整，无法全部替换
editor-replace-too-long = 匹配的文本过长，无法替换
editor-large-no-multi-cursor = 文件过大，不支持多光标查找

## 换行与折叠。

editor-word-wrap-on-large = 文件过大，无法自动换行
editor-word-wrap-on-pending = 已开启自动换行（大文件模式）
editor-word-wrap-on-table = 按列对齐的表格不支持自动换行
editor-word-wrap-on = 已开启自动换行
editor-word-wrap-off = 已关闭自动换行
editor-fold-large = 文件过大，无法折叠
editor-large-whole-file = 文件过大，无法对整个文件执行此操作

## 书签、符号、定义和用法。

editor-bookmark-added = 已添加书签
editor-bookmark-removed = 已移除书签
editor-bookmark-none = 此文件中没有书签
editor-bookmark-cleared = 已清除 { $count } 个书签
editor-no-open-file = 没有打开的文件
editor-symbol-function = 函数
editor-symbol-method = 方法
editor-symbol-type = 类型
editor-symbol-module = 模块
editor-symbol-variable = 变量
editor-symbol-heading = 标题
editor-symbol-section = 小节
editor-definition-no-name = 光标处没有可查找的名称
editor-usages-not-found = 未找到「{ $name }」的其他用法
editor-definition-at-caret = 光标处即为「{ $name }」的定义
editor-definition-not-found = 未找到「{ $name }」的定义
editor-open-folder-first = 请先打开一个目录
editor-places-definitions-count = { $count } 处定义
editor-places-usages-count = { $count } 个用法
editor-places-bookmarks-count = { $count } 个书签
editor-places-no-match = 没有匹配的位置
editor-places-usages-suffix = { " " }的用法
editor-places-definitions-suffix = { " " }的定义
editor-places-bookmarks = 书签

## 差异视图。

editor-diff-identical = 无差异
editor-diff-unchanged-lines = { $count } 行未更改
editor-diff-empty-file = 空文件

## 编码。

editor-encoding-gb18030 = 简体中文 (GB18030)
editor-encoding-gbk = 简体中文 (GBK)
editor-encoding-big5 = 繁体中文 (Big5)
editor-encoding-shift-jis = 日文 (Shift JIS)
editor-encoding-euc-jp = 日文 (EUC-JP)
editor-encoding-euc-kr = 韩文 (EUC-KR)
editor-encoding-windows-1252 = 西欧 (Windows 1252)
editor-encoding-mac-roman = 西欧 (Mac Roman)
editor-encoding-windows-1250 = 中欧 (Windows 1250)
editor-encoding-windows-1251 = 西里尔文 (Windows 1251)
editor-encoding-koi8-r = 西里尔文 (KOI8-R)
editor-encoding-windows-1253 = 希腊文 (Windows 1253)
editor-encoding-windows-1254 = 土耳其文 (Windows 1254)
editor-encoding-windows-1255 = 希伯来文 (Windows 1255)
editor-encoding-windows-1256 = 阿拉伯文 (Windows 1256)
editor-encoding-windows-1257 = 波罗的海文 (Windows 1257)
editor-encoding-windows-1258 = 越南文 (Windows 1258)
editor-encoding-windows-874 = 泰文 (Windows 874)
editor-encoding-lossy = 非 UTF-8
editor-encoding-binary = 二进制
editor-encoding-unencodable = 「{ $char }」无法以 { $encoding } 编码保存，请使用「通过编码保存」另存为 UTF-8

## 读写文件。

editor-file-no-path = 文件尚无路径
editor-original-changed = 磁盘上的文件在打开后已被其他程序修改，无法保存
editor-large-identity = 文件有多个硬链接或属于其他用户，大文件保存时会替换原文件，断开链接或改变属主，无法保存
editor-moved-while-saving = 文件在保存期间被移走了
editor-large-cannot-compare = 文件过大，无法比较
editor-large-utf8-only = 文件过大，只能按 UTF-8 打开
editor-file-is-binary = 此文件为二进制文件
editor-file-read-only = 文件是只读的
editor-temp-file-failed = 无法创建临时文件
editor-large-original-truncated = 原文件读取失败或已被截短，内容不完整
editor-paste-cannot-copy = 无法复制此项
editor-paste-into-itself = 无法复制到其自身内部
editor-open-binary = 此文件为二进制文件，无法打开
editor-open-too-large = 文件过大（{ $size } MB），超过 { $limit } GB 的上限
editor-unreadable-not-found = 文件不存在
editor-unreadable-permission = 没有读取权限
editor-unreadable-failed = 读取失败（{ $error }）
editor-unsupported-image = 图片
editor-unsupported-workbook = 工作簿
editor-unsupported-docx = Word 文档
editor-unsupported-pdf = PDF 文档
editor-unsupported-binary = 二进制文件或不支持的文本编码（{ $size }），无法显示预览
editor-unsupported-too-large = 文件过大（{ $size } GB，上限 { $limit } GB），无法显示预览
editor-unsupported-unreadable = { $reason }，无法显示预览
editor-size-bytes = { $count } 字节

## 外部程序。

editor-process-timeout = 超时
editor-process-too-large = 输出过多
editor-process-cancelled = 已取消
editor-process-no-stdout = 无法获取标准输出

## 只读和不能编辑的原因。

editor-readonly-pending = 文件尚未读取
editor-readonly-unsupported = 文件无法预览，不可编辑
editor-readonly-previewing = 正在预览，切换到源代码后才能编辑
editor-readonly-tab = 此标签页为只读
editor-readonly-binary = 二进制文件，已以只读方式打开
editor-readonly-long-lines = 文件中有超过 16 MB 的行，已以只读方式打开
editor-readonly-not-utf8 = 文件不是 UTF-8 编码，已以只读方式打开
editor-selection-too-large-to-edit = 选区超过 16 MB，不可编辑

## 复制和粘贴。

editor-selection-copied = 已复制所选内容
editor-selection-too-large-to-copy = 选区超过 { $size } MB，不能复制
editor-paste-copied-item = 已复制「{ $name }」
editor-paste-copied-items = 已复制 { $count } 项
editor-paste-cut-item = 已剪切「{ $name }」
editor-paste-cut-items = 已剪切 { $count } 项
editor-paste-copy-failed = 无法复制「{ $name }」：{ $error }
editor-paste-failed = 无法粘贴「{ $name }」：{ $error }
editor-paste-image-save-first = 先保存这个文件，图片会存到它所在的目录中
editor-paste-image-markdown-only = 图片只能粘贴到 Markdown 文件或文件树中
editor-paste-image-save-failed = 无法保存图片：{ $error }
editor-paste-clipboard-empty = 剪贴板是空的
editor-paste-copy-name = { $stem } 副本{ $ext }
editor-paste-copy-name-numbered = { $stem } 副本 { $number }{ $ext }
