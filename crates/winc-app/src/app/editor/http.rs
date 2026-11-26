//! `.http` 请求文件中发送请求：行号栏的运行按钮和快捷键找到光标处的请求，在后台用 `curl` 发送，
//! 响应显示在只读的「响应」标签页中。
//!
//! 请求的解析与发送见 `winc_editor::http`；同一时间只有一个请求在发送，再发送其他请求时取消前一个，
//! 再发送同一个请求即取消它。发送期间请求行的按钮转动，结束后显示结果并淡回常态。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use winc_editor::Document;
use winc_editor::http::{self, Request, Response, SendError};
use winc_editor::shell::Cancel;
use winc_i18n::t;

use super::*;

/// 正文超过该字节数（4 MiB）时，响应标签页只显示开头部分并注明总长度。
///
/// 取自调参：足够查看常见的接口返回，又不让编辑区为一次响应排版过多的行。
const SHOWN_BODY: usize = 4 << 20;

/// 请求结束后，请求行的按钮从结果（成功或失败）淡回常态的时长（1200 毫秒，取自调参）：
/// 足以看清结果，又不长时间停留。
const RESULT_FADE: Duration = Duration::from_millis(1200);

/// `.http` 请求的状态。
#[derive(Default)]
pub(crate) struct HttpState {
    /// 正在发送的请求；没有时为 `None`。
    running: Option<Running>,
    /// 刚结束的请求，其按钮正从结果淡回常态；关闭动画效果时没有。
    pub(in crate::app) finished: Option<Finished>,
    /// 各文档的请求行；绘制行号栏时读取，文档内容变化后重新计算。
    lines: RefCell<HashMap<DocumentId, RequestLines>>,
}

/// 一个文档的请求行：（计算时文档的版本号，请求行的行号）。
type RequestLines = (u64, Rc<[usize]>);

/// 正在发送的请求。
struct Running {
    /// 请求所在的文档。
    doc: DocumentId,
    /// 请求行的行号（从 0 起）。
    line: usize,
    /// 响应标签页的标题。
    title: String,
    /// 取消入口。
    cancel: Cancel,
    /// 后台线程送回结果的通道。
    receive: mpsc::Receiver<Result<Response, SendError>>,
}

/// 刚结束的请求。
pub(in crate::app) struct Finished {
    /// 请求所在的文档。
    doc: DocumentId,
    /// 请求行的行号（从 0 起）。
    line: usize,
    /// 是否成功：收到了状态码小于 400 的响应。
    ok: bool,
    /// 淡回常态的动画时钟。
    pub(in crate::app) clock: Clock,
}

/// 请求行的按钮在行号栏中的样子。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RunMark {
    /// 正在发送。
    Running,
    /// 刚结束：`ok` 为是否成功，`shown` 为结果图标的不透明度，从 1 降到 0。
    Done {
        /// 是否成功。
        ok: bool,
        /// 结果图标的不透明度。
        shown: f32,
    },
}

impl App {
    /// 文档 `doc` 中正在发送或刚结束的请求：（请求行的行号，按钮的样子）；没有时返回 `None`。
    pub(crate) fn http_mark(&self, doc: DocumentId) -> Option<(usize, RunMark)> {
        if let Some(running) = self.http.running.as_ref().filter(|r| r.doc == doc) {
            return Some((running.line, RunMark::Running));
        }
        let finished = self.http.finished.as_ref().filter(|f| f.doc == doc)?;
        let shown = finished.clock.between(1.0, 0.0);
        let mark = RunMark::Done {
            ok: finished.ok,
            shown,
        };
        Some((finished.line, mark))
    }

    /// 当前文档第 `line` 行的请求是否正在发送。
    pub(crate) fn http_sending(&self, line: usize) -> bool {
        let doc = self.editor.active().map(|d| d.id);
        (self.http.running.as_ref()).is_some_and(|r| Some(r.doc) == doc && r.line == line)
    }

    /// 请求按钮是否在动画中：发送中且开着动画效果时转动，结束后淡回常态。
    pub(in crate::app) fn http_animating(&self) -> bool {
        self.http.running.is_some() && self.animations_on() || self.http.finished.is_some()
    }

    /// 当前标签页是否为 `.http` 请求文件。
    pub(in crate::app) fn active_is_http(&self) -> bool {
        self.editor.active().is_some_and(is_request_file)
    }

    /// 文档 `doc` 是 `.http` 请求文件时返回其请求行的行号（从 0 起），否则返回 `None`。
    ///
    /// 按文档的版本号缓存：内容不变时不重新解析，可以在每帧的绘制中调用。
    pub(crate) fn http_request_lines(&self, doc: &Document) -> Option<Rc<[usize]>> {
        if !is_request_file(doc) {
            return None;
        }
        let mut lines = self.http.lines.borrow_mut();
        if let Some((revision, found)) = lines.get(&doc.id)
            && *revision == doc.revision
        {
            return Some(Rc::clone(found));
        }
        let found: Rc<[usize]> = http::request_lines(doc.text().rope()?).into();
        // 关闭的文档不再需要：只留仍打开的。
        lines.retain(|id, _| self.editor.document(*id).is_some());
        lines.insert(doc.id, (doc.revision, Rc::clone(&found)));
        Some(found)
    }

    /// 发送当前文档第 `line` 行（为 `None` 时是光标所在行）所在的请求，返回是否已处理。
    ///
    /// 当前文档不是 `.http` 请求文件时返回 `false`，快捷键留给其他命令。
    pub(in crate::app) fn send_http_request(&mut self, line: Option<usize>) -> bool {
        let Some(doc) = self.editor.active().filter(|_| self.active_is_http()) else {
            return false;
        };
        let line = line.unwrap_or_else(|| doc.text().line_of(doc.selection.head));
        let request = doc
            .text()
            .rope()
            .and_then(|rope| http::request_at(rope, line));
        let Some(request) = request else {
            self.flash(t!("editor-http-no-request"));
            return true;
        };
        let doc_id = doc.id;
        // 再发送正在发送的请求即取消它。
        let same = |r: &Running| r.doc == doc_id && r.line == request.line;
        if let Some(running) = self.http.running.take_if(|r| same(r)) {
            running.cancel.cancel();
            self.flash(t!("editor-http-cancelled"));
            return true;
        }
        if let Some(name) = undefined_variable(&request) {
            let message = t!("editor-http-undefined-variable", name = name);
            self.notify(NoticeKind::Error, message);
            return true;
        }
        if let Some(running) = self.http.running.take() {
            running.cancel.cancel();
        }
        self.http.finished = None;
        let line = request.line;
        let title = t!(
            "editor-http-response-title",
            request = format!("{} {}", request.method, request.url)
        );
        let cancel = Cancel::default();
        let (send, receive) = mpsc::channel();
        let proxy = self.link.proxy.clone();
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let result = http::send(&request, crate::web::curl(), &stop);
            if send.send(result).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::HttpDone);
            }
        });
        self.http.running = Some(Running {
            doc: doc_id,
            line,
            title,
            cancel,
            receive,
        });
        self.flash(t!("editor-http-sending"));
        true
    }

    /// 是否有请求正在发送；测试中没有事件循环，据此等待结果。
    #[cfg(test)]
    pub(in crate::app) fn http_running(&self) -> bool {
        self.http.running.is_some()
    }

    /// 接收后台送回的响应：在「响应」标签页中显示，失败时提示原因。
    pub(in crate::app) fn poll_http(&mut self) {
        let Some(running) = &self.http.running else {
            return;
        };
        let Ok(result) = running.receive.try_recv() else {
            return;
        };
        let Some(running) = self.http.running.take() else {
            return;
        };
        let ok = match &result {
            Ok(response) => response.status < 400,
            Err(SendError::Cancelled) => {
                self.request_redraw();
                return;
            }
            Err(_) => false,
        };
        let (doc, line) = (running.doc, running.line);
        self.http.finished = (self.clock(RESULT_FADE)).map(|clock| Finished {
            doc,
            line,
            ok,
            clock,
        });
        match result {
            Ok(response) => self.show_response(&running.title, &response),
            Err(SendError::Cancelled) => {}
            Err(e) => {
                let error = send_error(&e);
                self.notify(NoticeKind::Error, t!("editor-http-failed", error = error));
            }
        }
        self.request_redraw();
    }

    /// 在标题为 `title` 的只读标签页中显示响应 `response`，并在状态栏提示状态和用时。
    fn show_response(&mut self, title: &str, response: &Response) {
        let index = self.editor.show_output(title, &response_text(response));
        if let Some(doc) = self.editor.documents.get_mut(index) {
            doc.choose_language(winc_syntax::detect("response.http"));
        }
        self.activate_tab(index);
        let millis = response.elapsed.as_millis() as u64;
        let status = response.status_line.as_str();
        self.flash(t!("editor-http-received", status = status, millis = millis));
    }
}

/// 文档是否为 `.http` 请求文件：按 HTTP 着色、不是输出标签页。
///
/// 显示响应的标签页同样按 HTTP 着色，其中没有可发送的请求。
fn is_request_file(doc: &Document) -> bool {
    doc.title.is_none() && doc.highlighter.language().id == "http"
}

/// 请求中第一个没有定义的变量的名字；都已定义时返回 `None`。
fn undefined_variable(request: &Request) -> Option<String> {
    let parts = [request.url.as_str(), request.body.as_str()];
    let headers = request
        .headers
        .iter()
        .flat_map(|(n, v)| [n.as_str(), v.as_str()]);
    parts.into_iter().chain(headers).find_map(|text| {
        let open = text.find("{{")?;
        let close = text[open..].find("}}")?;
        Some(text[open + 2..open + close].trim().to_owned())
    })
}

/// 响应标签页的内容：状态行、响应头、空行和正文；正文不是 UTF-8 时只注明长度。
fn response_text(response: &Response) -> String {
    let mut out = response.status_line.clone();
    for (name, value) in &response.headers {
        out.push('\n');
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
    }
    out.push_str("\n\n");
    let size = response.body.len();
    let shown = &response.body[..size.min(SHOWN_BODY)];
    match std::str::from_utf8(shown) {
        Ok(body) => out.push_str(body),
        // 截断处可能落在多字节字符中间：只在截断时去掉末尾不完整的字符。
        Err(e) if size > SHOWN_BODY && e.error_len().is_none() => {
            out.push_str(&String::from_utf8_lossy(&shown[..e.valid_up_to()]));
        }
        Err(_) => out.push_str(&t!("editor-http-binary-body", size = size)),
    }
    if size > SHOWN_BODY {
        out.push('\n');
        out.push_str(&t!(
            "editor-http-body-truncated",
            shown = SHOWN_BODY,
            size = size
        ));
    }
    out
}

/// 发送失败的原因（给用户看的）。
pub(crate) fn send_error(error: &SendError) -> String {
    match error {
        SendError::Spawn(e) if e.kind() == std::io::ErrorKind::NotFound => {
            t!("editor-http-curl-missing").to_owned()
        }
        SendError::Spawn(e) => e.to_string(),
        SendError::Timeout => crate::web::Failure::Timeout.message(),
        SendError::TooLarge => t!("editor-http-too-large").to_owned(),
        SendError::Invalid => crate::web::Failure::Invalid.message(),
        SendError::Cancelled => String::new(),
        SendError::Curl { code, stderr } => {
            let proxied = winc_editor::proxy::configured(&crate::web::curl());
            crate::web::Failure::from_curl(*code, stderr, proxied).message()
        }
    }
}

#[cfg(test)]
mod tests;
