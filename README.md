<p align="center">
  <img src="./crates/winc-app/assets/icon.svg" alt="winc" width="128" />
</p>

<h1 align="center">Winc</h1>

<p align="center">
  A lighter, faster, more comfortable desktop editor.
</p>

<p align="center">
  English | <a href="./README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://winc.si"><img alt="Website" src="https://img.shields.io/badge/Website-winc.si-black" /></a>
  <a href="./LICENSE"><img alt="License" src="https://img.shields.io/badge/License-Apache%202.0-blue" /></a>
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-b7410e" />
  <img alt="Platforms" src="https://img.shields.io/badge/Platforms-macOS%20%7C%20Linux%20%7C%20Windows-555" />
</p>

<p align="center">
  <a href="https://winc.si/downloads"><img alt="Download Winc" src="./.github/assets/download-en.svg" /></a>
</p>

## Overview

winc is a desktop editor built with Rust and its own rendering engine. It starts in about a tenth
of a second, stays light with large files open, and responds to every keystroke and scroll at once.

| Measure | Result |
| --- | --- |
| Cold start to first frame | 95 ms |
| Memory with a 100 MB file open | 35 MB |
| Opening a 3.9 GB file with 41 million lines | 154 ms |
| Keystroke to frame | 1.0 ms |
| Frame time while scrolling | 0.6 ms |

Measured on macOS; results vary with hardware and system load.

## Features

| Area | Capabilities |
| --- | --- |
| Editing | Multiple cursors, find and replace, split panes, native input methods, smooth scrolling in gigabyte-sized files, and an experimental Vim mode |
| Syntax | Highlighting for common programming, markup, data, and configuration languages, with lightweight go-to-definition and find usages that need no language server |
| Formatting | Whole-file formatting through rustfmt, Prettier, Black, Ruff, gofmt, and other external tools |
| Preview | Markdown with math and Mermaid diagrams, images, SVG, PDF and LaTeX (macOS and Windows), Word documents, and spreadsheets, with the source one click away |
| Git | Side-by-side diffs with word-level highlights, all uncommitted changes in one tab, staging by hunk, committing selected lines, push, a commit graph, and blame |
| Project | A file tree, quick open, and project search that follows `.gitignore`, in the sidebar or a dialog, with results as a list or a tree |
| Terminal | An integrated terminal with split panes and tab groups that loads project Python environments automatically |
| Remote | Experimental SSH remote: edit, search, format, use Git, and open terminals in folders on Linux hosts; on Windows, open folders in WSL distributions |
| Interface | Sixteen built-in light and dark themes, and a complete interface in English and Simplified Chinese |
| Modules | Git, the terminal, and SSH remote can each be turned off; the start page sets up appearance, theme, language, and modules on first launch |
| Updates | Signed in-app updates on all three platforms, with the release notes shown after each update |

On touchscreens, tap to press buttons and place the caret, double-tap to select a word, swipe to
scroll, drag scrollbars and dividers, and hold for 500 ms to open the context menu. On Windows, the
title bar and window edges also respond to touch, with the same snapping as the mouse.

Image, SVG, and PDF previews zoom with Ctrl (⌘ on macOS) and the wheel, a pinch, or Ctrl/⌘ with
`+`, `-`, and `0`, keeping the point under the pointer in place. Scroll or drag to pan, and
double-click to fit the window again.

## Platforms

| Platform | Requirements | Package |
| --- | --- | --- |
| macOS | macOS 11 or later, Apple silicon or Intel | Disk image |
| Linux | x86_64 or ARM64, glibc 2.28 or later, X11 or Wayland | AppImage, deb, rpm, Arch repository |
| Windows | Windows 10 1809 or later, x86_64 | NSIS, MSI, portable ZIP, winget, Scoop |

Download winc from [winc.si](https://winc.si/downloads); every version is on
[GitHub Releases](https://github.com/DEEIX-AI/Winc/releases). winc is developed and tested mainly
on macOS.

On Windows, winc is also available through winget and Scoop. winget installs update in the app;
Scoop installs update with `scoop update winc`.

```powershell
winget install DEEIX-AI.Winc
scoop bucket add extras; scoop install extras/winc
```

On Arch Linux and Arch-based distributions such as CachyOS and Omarchy (x86_64), this command adds
the signed repository and installs winc, which then updates with `sudo pacman -Syu`.

```bash
curl --proto '=https' --tlsv1.2 -fLO https://winc.si/releases/arch/install-arch.sh && sudo bash install-arch.sh
```

The portable ZIP keeps settings and state in the `data` folder beside `winc.exe`. Without that
folder, they are kept in the user profile.

## Command Line

```bash
winc .                  # open the current folder
winc file.rs:12:5       # open a file at a line and column
winc --wait file        # wait until the tab closes, for example as the Git editor
cargo build 2>&1 | winc -   # read standard input
```

The Windows installers, winget, Scoop, and the deb, rpm, and Arch packages put `winc` on `PATH`;
the portable ZIP does not. On macOS, choose **Winc → Install winc Command…** in the menu bar or run
**Shell Command: Install winc Command** from the command palette; for the AppImage, use the command
palette.

On macOS the app is launched by the system and does not inherit the environment of your shell. Run
`winc --new-instance` to keep it.

## Building from Source

winc builds with Rust 1.99.1; `rust-toolchain.toml` selects the toolchain automatically.

```bash
cargo run --release                 # build and run
cargo run --release -- <path>       # open a file or folder
```

For versioning, signing configuration and release workflows, see [Releasing winc](release/README.md).

## Development

```bash
cargo xtask test                    # test the crates changed since HEAD
cargo xtask test --all              # test the whole workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo xtask bench                   # measure startup, memory, and scrolling against budgets
```

The interface is written in Rust. Windows come from winit, and rendering is done by winc's own
CPU rasterizer. On macOS, frames are written directly to an IOSurface.

The workspace is split into crates by responsibility:

| Crate | Responsibility |
| --- | --- |
| `winc-core` | Text, editing, undo, and diffing, independent of the interface |
| `winc-syntax` | Lexers for syntax highlighting |
| `winc-editor` | Editor logic: documents, file tree, search, Git, and formatting |
| `winc-vim` | Vim mode without the interface: key notation, the command table, parsing, and evaluation |
| `winc-backend` | File system access for local paths and for paths on remote hosts |
| `winc-server` | The server process that runs on a remote host and serves file operations over SSH |
| `winc-watch` | Watches folders on the machine that holds them (WSL distributions, remote hosts) |
| `winc-render` | Rendering: glyphs, rasterization, and presentation |
| `winc-preview` | Previews for images, SVG, Markdown, PDF, Word documents, and spreadsheets |
| `winc-typeset` | Layout for the Markdown preview |
| `winc-pty` | Terminal emulation and the pseudo-terminal helper process |
| `winc-config` | Configuration and state files |
| `winc-i18n` | Interface strings (Fluent bundles) |
| `winc-version` | The product version, read from the `VERSION` file |
| `winc-app` | The application: windows, events, and views |
| `winc` | The executable's entry point, which only calls into `winc-app` |
| `xtask` | Development automation: tests, benchmarks, and packaging |

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](.github/CONTRIBUTING.md) before you open a
pull request, and open it against the `dev` branch. All contributors sign the [Contributor License
Agreement](.github/CLA.md) and follow the [Code of Conduct](.github/CODE_OF_CONDUCT.md). To report a
vulnerability, see [SECURITY.md](.github/SECURITY.md).

## Contact

- Website: [winc.si](https://winc.si)
- Email: [support@deeix.com](mailto:support@deeix.com)
- Telegram: [t.me/deeix_chat](https://t.me/deeix_chat)
- QQ group: 260867503
- X: [@DEEIX_AI](https://x.com/DEEIX_AI)

## Links

- [Rust](https://www.rust-lang.org)
- [DEEIX AI](https://deeix.com)
- [LINUX DO](https://linux.do)

## License

winc is licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE).
