//! 临时打开模式，包括两种命令行用法。
//!
//! - `winc --wait 文件`：供 `git` 等程序把 winc 当作编辑器调用，等待的标签全部关闭后才退出。
//! - `winc -`：读取标准输入，边读边显示在「标准输入」标签中。
//!
//! 临时打开时不记录现场、不加入最近列表、不备份，不影响下次启动的状态。

use std::collections::HashSet;
use std::io::Read;
use std::sync::mpsc;

use super::*;
use crate::cli::Args;

/// 返回标准输入标签的标题。
pub fn stdin_title() -> &'static str {
    winc_i18n::t!("workspace-stdin-title")
}

/// 标准输入最多读取的字节数（256 MB，即 `256 << 20` 字节）。
///
/// 管道中的内容无法像文件那样按需读取，只能全部放进内存；达到上限后停止读取，并通过提示说明。
/// 该值为经验值，只限制内存占用。
const STDIN_LIMIT: usize = 256 << 20;

/// 临时打开模式的状态。
#[derive(Default)]
pub struct OneOff {
    /// `--wait` 等待的标签；标签全部关闭后退出。
    pub waiting: Option<HashSet<DocumentId>>,
    /// 等待的标签已全部关闭、退出尚未开始：处理完当前事件后退出（见 [`App::finish_waiting`]）。
    waited: bool,
    /// 正在后台读取的标准输入：标签对应的文档，以及接收各段内容的通道。
    ///
    /// 读取结束（或无法继续读取）后置为 `None`。
    stdin: Option<(DocumentId, mpsc::Receiver<Piece>)>,
}

/// 后台线程送回的一段标准输入。
enum Piece {
    Text(String),
    /// 读取结束；带原因时表示出错或达到 [`STDIN_LIMIT`]。
    Done(Option<String>),
}

impl App {
    /// 在启动时初始化临时打开模式：为 `--wait` 记录要等待的标签，为 `-` 开始读取标准输入。
    pub(crate) fn start_one_off(&mut self, args: &Args) {
        if args.stdin {
            let index = self.editor.show_output(stdin_title(), "");
            let id = self.editor.documents[index].id;
            self.one_off.stdin = Some((id, self.read_stdin()));
        }
        if args.wait {
            let ids = self.editor.documents.iter().map(|d| d.id).collect();
            self.one_off.waiting = Some(ids);
        }
    }

    /// 在后台线程中读取标准输入，分段通过返回的通道送回；每送出一段，唤醒界面线程。
    fn read_stdin(&self) -> mpsc::Receiver<Piece> {
        let (send, receive) = mpsc::channel();
        let wake = self.link.proxy.clone();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buf = vec![0u8; 64 << 10];
            let mut pending = Vec::new();
            let mut total = 0;
            let done = loop {
                let n = match stdin.read(&mut buf) {
                    Ok(0) => break None,
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        let error = e.to_string();
                        break Some(winc_i18n::t!("workspace-stdin-read-failed", error = error));
                    }
                };
                total += n;
                pending.extend_from_slice(&buf[..n]);
                // 末尾的多字节字符可能只读到前半部分；`error_len()` 为 `None` 表示数据在字符中途
                // 截断，这部分字节留在 `pending` 中，与下一段拼接后再解码。
                let complete = match std::str::from_utf8(&pending) {
                    Ok(_) => pending.len(),
                    Err(e) if e.error_len().is_none() => e.valid_up_to(),
                    Err(_) => pending.len(),
                };
                let text = String::from_utf8_lossy(&pending[..complete]).into_owned();
                pending.drain(..complete);
                if send.send(Piece::Text(text)).is_err() {
                    return;
                }
                if let Some(wake) = &wake {
                    let _ = wake.send_event(UserEvent::StdinRead);
                }
                if total >= STDIN_LIMIT {
                    let size = STDIN_LIMIT >> 20;
                    break Some(winc_i18n::t!("workspace-stdin-truncated", size = size));
                }
            };
            if !pending.is_empty() {
                let _ = send.send(Piece::Text(String::from_utf8_lossy(&pending).into_owned()));
            }
            let _ = send.send(Piece::Done(done));
            if let Some(wake) = &wake {
                let _ = wake.send_event(UserEvent::StdinRead);
            }
        });
        receive
    }

    /// 接收后台线程读到的标准输入，追加到标签末尾；光标原本在最后一行时随新内容下移。
    pub(super) fn poll_stdin(&mut self) {
        let Some((id, receive)) = &self.one_off.stdin else {
            return;
        };
        let id = *id;
        // 标签已关闭：停止接收并丢弃通道。后台线程下次发送时发现接收端已销毁，随即退出，
        // 因此像 `tail -f` 这类永不结束的输入不会让线程持续读取并反复唤醒界面。
        if self.editor.document(id).is_none() {
            self.one_off.stdin = None;
            return;
        }
        let mut text = String::new();
        let mut done = None;
        while let Ok(piece) = receive.try_recv() {
            match piece {
                Piece::Text(t) => text.push_str(&t),
                Piece::Done(problem) => done = Some(problem),
            }
        }
        if !text.is_empty()
            && let Some(doc) = self.editor.document_mut(id)
        {
            let follow = doc.selection.is_empty()
                && doc.text().line_of(doc.selection.head) + 1 == doc.text().line_count();
            doc.append_output(&text);
            if follow {
                doc.selection = Selection::point(doc.text().end_pos());
            }
            if follow && self.editor.active().is_some_and(|d| d.id == id) {
                self.relayout();
                self.ensure_caret_visible();
            }
        }
        if let Some(problem) = done {
            self.one_off.stdin = None;
            if let Some(problem) = problem {
                self.notify(problem);
            }
        }
        self.request_redraw();
    }

    /// 标签关闭后检查 `--wait` 等待的标签；全部关闭时记下，处理完当前事件后退出。
    pub(super) fn check_waiting(&mut self) {
        let Some(waiting) = &self.one_off.waiting else {
            return;
        };
        if waiting.iter().all(|id| self.editor.index_of(*id).is_none()) {
            self.one_off.waiting = None;
            self.one_off.waited = true;
        }
    }

    /// `--wait` 等待的标签全部关闭后，按 ⌘Q 的流程退出。
    ///
    /// 其他标签有未保存的修改时逐个询问（临时打开时不写备份），终端中有程序在运行时先确认；
    /// 用户取消后窗口保持打开，之后按正常流程退出。应用层在每个事件处理完后调用，因此退出在关闭
    /// 标签的流程（保存提示、另存为、关闭前的格式化）结束之后才开始。
    pub(crate) fn finish_waiting(&mut self) {
        if self.one_off.waited && !self.closing_pending() {
            self.one_off.waited = false;
            self.request_quit();
        }
    }
}

#[cfg(test)]
mod tests;
