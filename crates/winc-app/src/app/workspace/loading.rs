//! 在后台读取刚打开的文件。
//!
//! 打开文件时标签立即出现（尚未读取，见 `Document::is_pending`），读取、解码、建立行索引都在后台
//! 进行，读好后换入标签；界面线程不等待磁盘，耗时再长窗口仍然响应。
//!
//! 当前标签正在读取时先不绘制新的一帧，超过 [`HOLD`] 仍未读好才绘制「正在读取」，因为多数文件
//! 在两帧以内读完，界面上没有可见的停顿。之后要执行的动作先记下来，读好后由 [`App::when_loaded`]
//! 执行。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use winc_editor::LoadedFile;

use super::*;
use crate::app::tickets::Tickets;

/// 当前标签正在读取时，最多保留上一帧不绘制多久。
///
/// 取 100 ms：RAIL 模型把 100 ms 以内的响应视为即时；超过该时长仍未读好时先绘制「正在读取」，
/// 让用户知道读取正在进行、可以切走。
const HOLD: Duration = Duration::from_millis(100);

/// 文件读好之后要执行的回调。
type Then = Box<dyn FnOnce(&mut App)>;

/// 后台读取完成的结果：标签、读取代号、文件内容。
type LoadBack = (DocumentId, u64, LoadedFile);

/// 在后台读取的、刚打开的文件。
pub(crate) struct Loads {
    /// 各标签在等待的那一次读取；同一个标签同时只读取一次。
    pub(in crate::app) tickets: Tickets<DocumentId>,
    /// 后台读取结果的通道：发送端交给工作线程，接收端由界面线程轮询。
    pub(in crate::app) results: (mpsc::Sender<LoadBack>, mpsc::Receiver<LoadBack>),
    /// 各标签读好之后要执行的动作。
    pub(in crate::app) then: Vec<(DocumentId, Then)>,
    /// 当前标签开始读取的时刻：到 [`HOLD`] 之前不绘制新的一帧。
    pub(in crate::app) since: Option<(DocumentId, Instant)>,
    /// 是否因正在读取而跳过了一帧：读取完成或到达 [`HOLD`] 时重绘。
    pub(in crate::app) held: bool,
}

impl Default for Loads {
    fn default() -> Self {
        Self {
            tickets: Tickets::default(),
            results: mpsc::channel(),
            then: Vec::new(),
            since: None,
            held: false,
        }
    }
}

impl App {
    /// 当前标签尚未读取、也没有正在进行的读取时，交给后台读取。
    pub(in crate::app) fn load_active_in_background(&mut self) {
        let Some(job) = self.editor.active.and_then(|i| self.editor.begin_load(i)) else {
            return;
        };
        let id = job.id();
        if self.loads.tickets.is_waiting(&id) {
            return;
        }
        let ticket = self.loads.tickets.issue(id);
        self.loads.since = Some((id, Instant::now()));
        let sender = self.loads.results.0.clone();
        self.spawn_then(UserEvent::Loaded, move || {
            let _ = sender.send((id, ticket, job.run()));
        });
    }

    /// 收取后台读取完成的文件（见 [`Loads`]）并换入标签；结果属于当前标签时执行记录的动作。
    pub(in crate::app) fn poll_loads(&mut self) {
        while let Ok((id, ticket, loaded)) = self.loads.results.1.try_recv() {
            if !self.loads.tickets.redeem(&id, ticket) {
                continue;
            }
            if self.loads.since.is_some_and(|(s, _)| s == id) {
                self.loads.since = None;
            }
            let Some(index) = self.editor.finish_load(loaded) else {
                continue;
            };
            // 未读取的标签不参与磁盘同步，读取期间文件可能再次被修改：读完后按路径同步一次元数据。
            if let Some(path) = self.editor.documents[index].path.clone() {
                self.sync_documents(|p| p == path);
            }
            // 不是当前标签时记录的动作保留，再次切到该标签时由 `run_when_loaded` 执行。
            if self.editor.active == Some(index) {
                self.loaded_active();
                self.run_when_loaded();
            }
        }
        // 已关闭的标签不再等待读取结果。
        let editor = &self.editor;
        self.loads
            .then
            .retain(|(id, _)| editor.index_of(*id).is_some());
    }

    /// 当前标签刚读取完成：重新布局、提示只读的原因、保证光标可见并请求重绘。
    fn loaded_active(&mut self) {
        self.sticky_cache = None;
        self.relayout();
        self.announce_readonly();
        self.ensure_caret_visible();
        self.request_redraw();
    }

    /// 当前标签读取完成后执行 `then`；当前标签已经读好时立即执行。
    ///
    /// 跳转、选中都针对当前标签：读取完成时已切到其他标签的（例如命令行一次打开多个带行号的
    /// 文件），留到再次切到该标签时执行；标签已关闭的不执行。
    pub(in crate::app) fn when_loaded(&mut self, then: impl FnOnce(&mut App) + 'static) {
        match self
            .editor
            .active()
            .filter(|d| d.is_pending())
            .map(|d| d.id)
        {
            Some(id) => self.loads.then.push((id, Box::new(then))),
            None => then(self),
        }
    }

    /// 当前标签已经读好时，执行为它记录的、尚未执行的动作（见 [`App::when_loaded`]）。
    ///
    /// 读好时切走了的标签，动作在这里补上：切换标签的路径很多，因此在每帧绘制之前调用。
    pub(in crate::app) fn run_when_loaded(&mut self) {
        let Some(id) = self
            .editor
            .active()
            .filter(|d| !d.is_pending())
            .map(|d| d.id)
        else {
            return;
        };
        for (_, then) in extract(&mut self.loads.then, |(i, _)| *i == id) {
            then(self);
        }
    }

    /// 打开文件 `path`，读好以后执行 `then`（见 [`App::when_loaded`]）。
    ///
    /// 换了标签时，`then` 中的跳转直接到位，不做平滑滚动：平滑滚动用于在同一文档内保持位置感，
    /// 刚换来的文档没有可保持的位置，动画只会推迟目标行出现。
    pub(in crate::app) fn open_path_then(
        &mut self,
        path: &Path,
        then: impl FnOnce(&mut App) + 'static,
    ) {
        self.open_then(path, false, then);
    }

    /// 同 [`Self::open_path_then`]，启用预览标签时在预览标签中打开（见 [`App::preview_path`]）。
    pub(in crate::app) fn preview_path_then(
        &mut self,
        path: &Path,
        then: impl FnOnce(&mut App) + 'static,
    ) {
        self.open_then(path, self.preview_on_click(), then);
    }

    fn open_then(&mut self, path: &Path, preview: bool, then: impl FnOnce(&mut App) + 'static) {
        let from = self.editor.active().map(|d| d.id);
        if preview {
            self.preview_path(path);
        } else {
            self.open_path(path);
        }
        let switched = self.editor.active().map(|d| d.id) != from;
        self.when_loaded(move |app| {
            then(app);
            if switched {
                app.scroll_glide = None;
            }
        });
    }

    /// 当前标签正在读取、且尚未到 [`HOLD`] 时，返回可以跳过绘制的截止时刻。
    pub(in crate::app) fn holding_frame(&self, now: Instant) -> Option<Instant> {
        let (id, since) = self.loads.since?;
        let active = self.editor.active()?;
        let until = since + HOLD;
        (active.id == id && active.is_pending() && now < until).then_some(until)
    }
}

/// 从 `items` 中取出 `take` 返回 `true` 的元素，按原来的顺序返回。
fn extract<T>(items: &mut Vec<T>, mut take: impl FnMut(&T) -> bool) -> Vec<T> {
    let (taken, kept) = std::mem::take(items).into_iter().partition(|t| take(t));
    *items = kept;
    taken
}
