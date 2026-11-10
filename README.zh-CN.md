<p align="center">
  <img src="./crates/winc-app/assets/icon.svg" alt="winc" width="128" />
</p>

<h1 align="center">Winc</h1>

<p align="center">
  更轻、更快、更顺手的桌面编辑器。
</p>

<p align="center">
  <a href="./README.md">English</a> | 简体中文
</p>

<p align="center">
  <a href="https://winc.si"><img alt="Website" src="https://img.shields.io/badge/Website-winc.si-black" /></a>
  <a href="./LICENSE"><img alt="License" src="https://img.shields.io/badge/License-Apache%202.0-blue" /></a>
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-b7410e" />
  <img alt="Platforms" src="https://img.shields.io/badge/Platforms-macOS%20%7C%20Linux%20%7C%20Windows-555" />
</p>

<p align="center">
  <a href="https://winc.si/#download"><img alt="下载 Winc" src="./.github/assets/download-zh.svg" /></a>
</p>

## 概述

winc 以 Rust 与自研渲染引擎构建：0.1 秒启动，十万行代码仅占 52 MB 内存。多光标、查找替换、并排差异，
每一次输入与滚动都即时响应。

| 指标 | 结果 |
| --- | --- |
| 冷启动到第一帧 | 119 ms（20 次的中位数） |
| 打开 10 万行时的常驻内存 | 52 MB |
| 1 GB 的文件，可编辑、有高亮 | 220 ms |

在 macOS 上测得，随硬件和系统负载略有不同。

## 功能

| 方面 | 能力 |
| --- | --- |
| 编辑 | 多光标、查找替换、系统输入法，GB 级文件中流畅滚动 |
| 语法 | 常见编程语言、标记语言、数据和配置格式的高亮，不依赖语言服务器的轻量跳转定义和查找用法 |
| 格式化 | 调用 rustfmt、Prettier、Black、Ruff、gofmt 等外部工具整理整个文件 |
| 预览 | Markdown、图片、SVG、表格（xlsx、csv），随时切回源代码 |
| Git | 并排差异（词级高亮）、按块暂存、提交和推送、提交图、逐行追溯 |
| 项目 | 文件树、快速打开、遵循 `.gitignore` 的项目搜索 |
| 终端 | 窗口底部的内置终端 |
| 界面 | 浅色和深色主题，完整的英文和简体中文界面 |
| 更新 | 三个平台都有签名的应用内更新 |

触摸屏支持点按按钮和定位光标、双击选词、单指滑动滚动，以及拖动滚动条和面板分隔条。
Windows 上还可拖动标题栏移动窗口、双击标题栏最大化或还原，以及拖动窗口边缘改变大小。
窗口拖动和边缘吸附与鼠标操作共用 Windows 原生行为。
长按 500 毫秒打开当前位置的右键菜单。滑动始终作用于落指的区域，抬手后停止；多指接触会取消
当前手势，所有手指离开后可继续操作。

## 平台

| 平台 | 要求 | 安装包 |
| --- | --- | --- |
| macOS | macOS 11 及以上，Apple 芯片或 Intel | 磁盘映像 |
| Linux | x86_64，X11 或 Wayland | AppImage、deb |
| Windows | Windows 10 1809 及以上，x86_64 | NSIS、MSI、便携 ZIP |

便携 ZIP 将设置和状态保存在 `winc.exe` 旁边的 `data` 目录中；删除该目录后改为保存在用户目录中。

主要在 macOS 上开发和测试。可从 [winc.si](https://winc.si/#download) 下载，所有版本见
[GitHub Releases](https://github.com/DEEIX-AI/Winc/releases)。

## 命令行

```bash
winc .                  # 打开当前目录
winc file.rs:12:5       # 打开文件并跳到指定的行和列
winc --wait file        # 等标签关闭后再退出，例如作为 Git 的编辑器
```

Windows 安装包、Linux deb 包和 Arch 包会把 `winc` 加入 `PATH`；Windows 便携版不修改 `PATH`。macOS 请在菜单栏选择「Winc → 安装 winc 命令…」，或在命令面板中运行「Shell 命令：安装 winc 命令」；Linux AppImage 请使用命令面板。

在 macOS 上应用由系统启动，不继承 shell 的环境变量；需要保留时使用 `winc --new-instance`。

## 从源代码构建

需要 Rust 1.99.1，`rust-toolchain.toml` 会自动选用工具链。

```bash
cargo run --release                 # 构建并运行
cargo run --release -- <路径>        # 打开文件或目录
```

版本同步、签名配置和发布流程见[发布说明](release/README.md)。

## 开发

```bash
cargo xtask test                    # 只测和 HEAD 相比改动涉及的 crate
cargo xtask test --all              # 测整个 workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo xtask bench                   # 测启动、内存、滚动，对照性能预算
```

界面由 Rust 编写，窗口用 winit，渲染由 winc 自己的 CPU 光栅化完成；在 macOS 上直接写入 IOSurface。

代码按 crate 分层：

| crate | 职责 |
| --- | --- |
| `winc-core` | 文本、编辑、撤销、差异等与界面无关的基础 |
| `winc-syntax` | 语法高亮的词法分析 |
| `winc-editor` | 文档、文件树、查找、Git、格式化等编辑器逻辑 |
| `winc-render` | 渲染层：字形、光栅化、呈现 |
| `winc-preview` | 图片、SVG、Markdown、表格预览 |
| `winc-typeset` | Markdown 预览的排版 |
| `winc-pty` | 终端仿真和伪终端的辅助进程 |
| `winc-config` | 配置和状态文件 |
| `winc-i18n` | 界面文案（Fluent 语言包） |
| `winc-version` | 产品版本号，取自 `VERSION` 文件 |
| `winc-app` | 应用本体：窗口、事件、各个界面 |
| `winc` | 可执行文件的入口，只调用 `winc-app` |
| `xtask` | 开发自动化：测试、基准、打包 |

## 参与贡献

欢迎贡献。提交合并请求前请阅读 [CONTRIBUTING.md](.github/CONTRIBUTING.md)，合并请求请提到
`dev` 分支。所有贡献者都要签署[贡献者许可协议](.github/CLA.md)，并遵守
[行为准则](.github/CODE_OF_CONDUCT.md)。报告安全漏洞见 [SECURITY.md](.github/SECURITY.md)。

## 联系我们

- 网站：[winc.si](https://winc.si)
- 邮箱：[support@deeix.com](mailto:support@deeix.com)
- Telegram：[t.me/deeix_chat](https://t.me/deeix_chat)
- X：[@DEEIX_AI](https://x.com/DEEIX_AI)

## 友情链接

- [Rust](https://www.rust-lang.org)
- [DEEIX AI](https://deeix.com)
- [LINUX DO](https://linux.do)

## 许可证

winc 以 [Apache License 2.0](LICENSE) 授权，另见 [NOTICE](NOTICE)。
