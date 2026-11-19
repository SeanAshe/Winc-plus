//! 终端里的链接：按住 ⌘ 时认鼠标所在那一行里的 OSC 8 超链接、网址和存在的文件
//! （`路径:行:列`，也认不带行号的路径），画下划线；⌘ 单击打开：网址交给系统，文件在
//! 编辑区打开到那一行那一列，⌘⇧ 单击在文件树里定位。
//!
//! 只看鼠标所在的一行（网址连同折行的相邻行），不扫整屏和回滚缓冲。相对路径按那一行所在的
//! 命令运行时的目录解析（命令块记着 OSC 7 报的目录），没有块时用 shell 最近报的目录，再没有
//! 用打开的目录。

use std::path::{Path, PathBuf};

use super::App;
use super::blocks::path_from_bytes;
use super::locate::{locations, url_across};
use super::tab::TermTab;
use crate::app::NoticeKind;
use crate::system;
use winc_pty::protocol::Line;

/// 链接指向哪里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// 网址，交给系统打开。
    Url(String),
    /// 文件或目录；行号、列号从 1 起。
    File {
        path: PathBuf,
        line: Option<usize>,
        col: Option<usize>,
    },
}

/// 按住 ⌘ 时鼠标下的链接。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TermLink {
    /// 链接所在的标签。
    pub tab: u16,
    /// 链接所在的绝对行号。
    pub line: u64,
    /// 链接在这一行里从第几格开始。
    pub start: usize,
    /// 链接的结束格（不含）。
    pub end: usize,
    /// 链接指向哪里。
    pub target: Target,
}

/// `file://主机/路径` 里的路径。
fn file_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    Some(PathBuf::from(percent_decode(path)))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 把终端里的路径换成本机路径：打开的目录位于 WSL 发行版中时，绝对路径属于同一发行版。
fn in_project(root: Option<&Path>, path: PathBuf) -> PathBuf {
    match root {
        Some(root) => super::helper::terminal_path(root, path),
        None => path,
    }
}

/// 向前、向后各查看的折行数上限：128 行在 80 列时约 1 万个字符，超过常见网址的长度上限；
/// 限制它避免在很长的单行输出上逐行回溯。
const WRAP_ROWS: usize = 128;

/// 第 `line` 行所在的逻辑行：折行连在一起的各显示行，以及第 `line` 行在其中的下标。
///
/// 回滚缓冲中尚未取回的行截断逻辑行。
fn logical_rows(tab: &TermTab, line: u64) -> (Vec<&Line>, usize) {
    let wraps = |l: u64| tab.line(l).is_some_and(|row| row.wrapped);
    let mut first = line;
    while first > tab.history.0 && line - first < WRAP_ROWS as u64 && wraps(first - 1) {
        first -= 1;
    }
    let mut rows = Vec::new();
    let mut at = first;
    while let Some(row) = tab.line(at) {
        rows.push(row);
        if !row.wrapped || at >= line + WRAP_ROWS as u64 {
            break;
        }
        at += 1;
    }
    (rows, (line - first) as usize)
}

/// 第 `line` 行第 `col` 格上的链接。
fn link_at(
    tab: &TermTab,
    line: u64,
    col: usize,
    root: Option<&Path>,
    exists: &dyn Fn(&Path, bool) -> bool,
) -> Option<(usize, usize, Target)> {
    let text = tab.line(line)?;
    // OSC 8：样式里带着链接编号的那一段格子。
    let mut runs = text.runs.iter().peekable();
    while let Some(run) = runs.next() {
        let start = usize::from(run.start);
        let end = runs
            .peek()
            .map_or(usize::from(tab.size.cols), |r| usize::from(r.start));
        if run.style.link != 0
            && (start..end).contains(&col)
            && let Some(uri) = tab.link(run.style.link)
        {
            let target = match file_url(uri) {
                Some(path) => Target::File {
                    path: in_project(root, path),
                    line: None,
                    col: None,
                },
                None if system::openable_url(uri) => Target::Url(uri.clone()),
                // 其他协议不作为链接：显示的文字可以与实际地址不同。看起来像网址的文字由后面的
                // 代码按显示的内容识别。
                None => break,
            };
            return Some((start, end, target));
        }
    }
    // 网址按折行连在一起的逻辑行识别：超过面板宽度的网址取完整的地址。
    let (rows, at) = logical_rows(tab, line);
    if let Some((start, end, url)) = url_across(&rows, at, col) {
        let target = match file_url(&url) {
            Some(path) => Target::File {
                path: in_project(root, path),
                line: None,
                col: None,
            },
            None => Target::Url(url),
        };
        return Some((start, end, target));
    }
    let found = locations(text, true)
        .into_iter()
        .find(|l| (l.start..l.end).contains(&col))?;
    let path = in_project(root, PathBuf::from(&found.path));
    let path = if path.is_absolute() {
        path
    } else {
        let block = tab.block_at(line).map(|i| &tab.blocks[i]);
        let cwd = block
            .map(|b| &b.cwd)
            .filter(|c| !c.is_empty())
            .or(Some(&tab.cwd).filter(|c| !c.is_empty()))
            .map(|c| in_project(root, path_from_bytes(c)))
            .or_else(|| root.map(Path::to_path_buf))?;
        cwd.join(path)
    };
    exists(&path, found.line.is_some()).then_some((
        found.start,
        found.end,
        Target::File {
            path,
            line: found.line,
            col: found.col,
        },
    ))
}

impl App {
    /// 鼠标动了或者修饰键变了：按住 ⌘（别的系统上是 Ctrl）、鼠标在网格上时认鼠标下的链接。
    pub(in crate::app) fn update_terminal_link(&mut self) {
        let held = if cfg!(target_os = "macos") {
            self.pointer.modifiers.super_key()
        } else {
            self.pointer.modifiers.control_key()
        };
        let (x, y) = self.pointer.mouse;
        let view = self.terminal_view();
        let root = self.editor.root().map(Path::to_path_buf);
        // 吸顶的命令行、折起的那一行底下的字看不见，不认链接。
        let link = if held && view.contains(x, y) && self.terminal_shows_line(y) {
            self.terminal_point(x, y, false).and_then(|point| {
                let tab = self.terminal.tab()?;
                let exists = |path: &Path, numbered| self.terminal_target(path, numbered);
                let root = root.as_deref();
                let (start, end, target) = link_at(tab, point.line, point.col, root, &exists)?;
                Some(TermLink {
                    tab: tab.id,
                    line: point.line,
                    start,
                    end,
                    target,
                })
            })
        } else {
            None
        };
        if link != self.terminal.link {
            self.terminal.link = link;
            self.update_cursor_icon();
            self.request_redraw();
        }
    }

    /// ⌘ 单击：打开鼠标下的链接（`reveal` 为真时文件在文件树里定位）。
    ///
    /// 有链接时返回真。
    pub(in crate::app) fn open_terminal_link(&mut self, reveal: bool) -> bool {
        self.update_terminal_link();
        let Some(link) = self.terminal.link.take() else {
            return false;
        };
        match link.target {
            Target::Url(url) => {
                if let Err(e) = system::open_url(&url) {
                    self.notify(
                        NoticeKind::Error,
                        winc_i18n::t!("terminal-link-open-failed", error = e.to_string()),
                    );
                }
            }
            Target::File { path, .. } if reveal || self.is_folder(&path) => {
                self.reveal_path(&path);
            }
            Target::File { path, line, col } => {
                self.open_path(&path);
                if let Some(line) = line {
                    self.jump_to(line - 1, col.unwrap_or(1) - 1, 0);
                }
            }
        }
        self.request_redraw();
        true
    }
}

#[cfg(test)]
mod tests;
