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
  <a href="https://winc.si/#download"><img alt="Download Winc" src="./.github/assets/download-en.svg" /></a>
</p>

## Overview

winc is built with Rust and its own rendering engine: it starts in 0.1 seconds and uses just 52 MB
of memory with 100,000 lines open. Multiple cursors, find and replace, and side-by-side diffs are
all there, and every keystroke and scroll responds instantly.

| Measure | Result |
| --- | --- |
| Cold start to first frame | 119 ms (median of 20 runs) |
| Resident memory with 100,000 lines open | 52 MB |
| A 1 GB file, editable and highlighted | 220 ms |

Measured on macOS; results vary with hardware and system load.

## Features

| Area | Capabilities |
| --- | --- |
| Editing | Multiple cursors, find and replace, native input methods, and smooth scrolling in gigabyte-sized files |
| Syntax | Highlighting for common programming, markup, data, and configuration languages, plus lightweight go-to-definition and usages without a language server |
| Formatting | Whole-file formatting with external tools such as rustfmt, Prettier, Black, Ruff, and gofmt |
| Preview | Markdown, images, SVG, and spreadsheets (xlsx, csv), with the source one click away |
| Git | Side-by-side diffs with word-level highlights, staging by hunk, commit and push, a commit graph, and blame |
| Project | A file tree, quick open, and project search that follows `.gitignore` |
| Terminal | An integrated terminal at the bottom of the window |
| Interface | Light and dark themes, and a complete interface in English and Simplified Chinese |
| Updates | Signed in-app updates on all three platforms |

Touchscreens support tapping buttons and placing the caret, double-tapping to select a word,
swiping to scroll, and dragging scrollbars and panel dividers. Hold for 500 ms to open the context
menu. Swipes stay in the region where they started and stop on release. Additional fingers cancel
the current gesture; lift all fingers before starting another. On Windows, drag the title bar to
move the window, double-tap it to maximize or restore, and drag the window edges to resize.
Window dragging and edge snapping use the same native Windows behavior as mouse input.

## Platforms

| Platform | Requirements | Package |
| --- | --- | --- |
| macOS | macOS 11 or later, Apple silicon or Intel | Disk image |
| Linux | x86_64, X11 or Wayland | AppImage, deb |
| Windows | Windows 10 1809 or later, x86_64 | NSIS, MSI, portable ZIP |

The portable ZIP keeps settings and state in the `data` folder beside `winc.exe`; delete that
folder to keep them in the user profile instead.

winc is developed and tested mainly on macOS. Download it from
[winc.si](https://winc.si/#download); all versions are on
[GitHub Releases](https://github.com/DEEIX-AI/Winc/releases).

## Command Line

```bash
winc .                  # open the current folder
winc file.rs:12:5       # open a file at a line and column
winc --wait file        # wait until the tab closes, for example as the Git editor
```

The Windows installers and the Linux deb and Arch packages add `winc` to `PATH`; the Windows portable ZIP leaves `PATH` unchanged. On macOS, choose **Winc → Install winc Command…** from the menu bar or run **Shell Command: Install winc Command** from the command palette; with the Linux AppImage, use the command palette.

On macOS the app is started by the system, so it does not inherit the environment variables of your shell. Use `winc --new-instance` to keep them.

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
| `winc-render` | Rendering: glyphs, rasterization, and presentation |
| `winc-preview` | Previews for images, SVG, Markdown, and spreadsheets |
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
- X: [@DEEIX_AI](https://x.com/DEEIX_AI)

## Links

- [Rust](https://www.rust-lang.org)
- [DEEIX AI](https://deeix.com)
- [LINUX DO](https://linux.do)

## License

winc is licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE).
