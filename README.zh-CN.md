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
  <a href="https://winc.si/zh-cn/downloads"><img alt="下载 Winc" src="./.github/assets/download-zh.svg" /></a>
</p>

## 概述

winc 是以 Rust 和自研渲染引擎构建的桌面编辑器：约 0.1 秒启动，打开大文件也保持轻量，每一次输入和滚动
都即时响应。

| 指标 | 结果 |
| --- | --- |
| 冷启动到第一帧 | 95 ms |
| 打开 100 MB 文件时的内存占用 | 35 MB |
| 打开 3.9 GB、4100 万行的文件 | 154 ms |
| 按键到画面更新 | 1.0 ms |
| 滚动时每帧耗时 | 0.6 ms |

在 macOS 上测得，随硬件和系统负载略有不同。

## 功能

| 方面 | 能力 |
| --- | --- |
| 编辑 | 多光标、查找替换、拆分窗格、系统输入法、GB 级文件中流畅滚动，以及实验性的 Vim 模式 |
| 语法 | 常见编程语言、标记语言、数据和配置格式的高亮，以及不依赖语言服务器的跳转定义和查找用法 |
| 格式化 | 调用 rustfmt、Prettier、Black、Ruff、gofmt 等外部工具格式化整个文件 |
| 预览 | Markdown（含公式和 Mermaid 图）、图片、SVG、PDF 和 LaTeX（macOS 和 Windows）、Word 文档、表格，随时切回源代码 |
| Git | 并排差异（词级高亮）、在一个标签页中查看全部未提交的更改、按块暂存、按行勾选提交、推送、提交图、逐行追溯 |
| 项目 | 文件树、快速打开、遵循 `.gitignore` 的项目搜索，可在侧栏或对话框中使用，结果以列表或树形显示 |
| 终端 | 内置终端，支持拆分窗格和标签页组，自动加载项目内的 Python 虚拟环境 |
| 远程 | 实验性的 SSH 远程：在 Linux 主机的目录中编辑、搜索、格式化、使用 Git 和终端；Windows 上可打开 WSL 发行版中的目录 |
| 界面 | 十六套内置的浅色和深色主题，完整的英文和简体中文界面 |
| 模块 | Git、终端和 SSH 远程均可单独关闭；首次启动时在开始页设置外观、主题、语言和模块 |
| 更新 | 三个平台都有签名的应用内更新，更新后可在应用内查看更新日志 |

触摸屏上可点按按钮和定位光标、双击选词、滑动滚动、拖动滚动条和分隔条，长按 500 毫秒打开右键菜单。
Windows 上的标题栏和窗口边缘也支持触摸操作，吸附效果与鼠标一致。

图片、SVG 和 PDF 预览可按住 Ctrl（macOS 为 ⌘）滚动、双指捏合，或按 Ctrl/⌘ 加 `+`、`-`、`0` 缩放，
鼠标所指的位置保持不动。放大后滚动或拖动查看，双击恢复适应窗口。

## 平台

| 平台 | 要求 | 安装包 |
| --- | --- | --- |
| macOS | macOS 11 及以上，Apple 芯片或 Intel | 磁盘映像 |
| Linux | x86_64 或 ARM64，glibc 2.28 及以上，X11 或 Wayland | AppImage、deb、rpm、Arch 软件源 |
| Windows | Windows 10 1809 及以上，x86_64 | NSIS、MSI、便携 ZIP、winget、Scoop |

可从 [winc.si](https://winc.si/zh-cn/downloads) 下载，所有版本见
[GitHub Releases](https://github.com/DEEIX-AI/Winc/releases)。winc 主要在 macOS 上开发和测试。

Windows 上也可以通过 winget 或 Scoop 安装。winget 安装的版本在应用内更新，Scoop 安装的版本用
`scoop update winc` 更新。

```powershell
winget install DEEIX-AI.Winc
scoop bucket add extras; scoop install extras/winc
```

在 Arch Linux 及 CachyOS、Omarchy 等基于 Arch 的发行版（x86_64）上，以下命令添加经过签名的软件源并
安装 winc，之后随 `sudo pacman -Syu` 更新。

```bash
curl --proto '=https' --tlsv1.2 -fLO https://winc.si/releases/arch/install-arch.sh && sudo bash install-arch.sh
```

便携 ZIP 把设置和状态保存在 `winc.exe` 旁边的 `data` 目录中，没有该目录时保存在用户目录中。

## 命令行

```bash
winc .                  # 打开当前目录
winc file.rs:12:5       # 打开文件并跳到指定的行和列
winc --wait file        # 等标签页关闭后再退出，例如作为 Git 的编辑器
cargo build 2>&1 | winc -   # 读取标准输入
```

Windows 安装包、winget、Scoop，以及 deb、rpm 和 Arch 包会把 `winc` 加入 `PATH`，便携 ZIP 不会。
macOS 上在菜单栏选择「Winc → 安装 winc 命令…」，或在命令面板中运行「Shell 命令：安装 winc 命令」；
AppImage 请使用命令面板。

macOS 上的应用由系统启动，不继承 shell 的环境变量；需要保留时使用 `winc --new-instance`。

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
| `winc-vim` | Vim 模式的无界面部分：按键记法、命令表、命令的解析与求值 |
| `winc-backend` | 文件系统的访问入口，区分本机路径与远端主机上的路径 |
| `winc-server` | 运行在远端主机上的服务进程，经 SSH 为本机执行文件操作 |
| `winc-watch` | 在文件所在的机器上监视目录（WSL 发行版内、远端主机上） |
| `winc-render` | 渲染层：字形、光栅化、呈现 |
| `winc-preview` | 图片、SVG、Markdown、PDF、Word 文档、表格预览 |
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
- QQ 群：260867503
- X：[@DEEIX_AI](https://x.com/DEEIX_AI)

<img src="./.github/assets/qq-group.jpg" alt="QQ 群 260867503 的二维码" width="200" />

## 友情链接

- [Rust](https://www.rust-lang.org)
- [DEEIX AI](https://deeix.com)
- [LINUX DO](https://linux.do)

## 许可证

winc 以 [Apache License 2.0](LICENSE) 授权，另见 [NOTICE](NOTICE)。
