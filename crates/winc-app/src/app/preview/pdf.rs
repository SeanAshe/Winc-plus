//! PDF 预览的状态：渲染线程、各页大小、渲染好的页和滚动位置。
//!
//! 每个正在显示的 PDF 有一个渲染线程：文档在该线程中打开（系统的 PDF 对象不能跨线程使用），按
//! 请求逐页渲染。绘制时只请求视口内和视口附近的页（`App::request_pdf_pages`）：不再需要的页从
//! 缓存中移除，线程中排队但已不需要的请求跳过，内存只占这几页的位图。切换标签或文件被修改时
//! 结束线程。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, mpsc};

use winc_editor::{DocumentId, Unsupported};
use winc_preview::image::Pixels;
use winc_preview::pdf::{Document, PageSize};
use winc_render::Rect;
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, UserEvent};

/// 方向键每次滚动的距离（逻辑像素）：约为正文三行的高度。
const ARROW_STEP: f32 = 48.0;

/// 渲染线程的请求：页号（从 0 起）和位图大小（物理像素）。
struct Request {
    page: usize,
    width: u32,
    height: u32,
}

/// 渲染线程送回的结果，带有打开时的序号：序号与当前序号不同时说明文档已经更换，结果丢弃。
enum Done {
    /// 打开的结果：各页大小，或失败的原因。
    Opened(u64, Result<Vec<PageSize>, String>),
    /// 渲染完的一页：页号、请求的位图宽度，以及位图或失败的原因。
    Rendered(u64, usize, u32, Result<Arc<Pixels>, String>),
}

/// 当前标签的 PDF 预览。
pub(crate) struct PdfPreview {
    /// 预览的文档及其版本号（`revision`）；文档或版本改变时重新打开。
    doc: Option<(DocumentId, u64)>,
    /// 当前打开的序号，每次重新打开加一，用来丢弃旧线程送回的结果。
    generation: u64,
    /// 向渲染线程发请求；丢弃时线程结束，文档随之关闭。
    requests: Option<mpsc::Sender<Request>>,
    /// 当前需要的（页号，位图宽度）：渲染线程开始渲染前检查，不在其中的请求跳过。
    wanted: Arc<Mutex<HashSet<(usize, u32)>>>,
    /// 已发给渲染线程、尚未送回的（页号，位图宽度）。
    requested: HashSet<(usize, u32)>,
    sender: mpsc::Sender<Done>,
    receiver: mpsc::Receiver<Done>,
    /// 打开的结果：各页大小（点），或失败的原因；尚未打开完时为 `None`。
    pub pages: Option<Result<Arc<[PageSize]>, String>>,
    /// 渲染好的页：页号到位图，只保留当前需要的页。
    pub rendered: HashMap<usize, Arc<Pixels>>,
    /// 渲染失败的（页号，位图宽度）和原因：在页面上显示原因，不再重复请求；位图宽度改变（窗口
    /// 缩放）或重新打开文档时重试。
    pub failed: HashMap<(usize, u32), String>,
    /// 滚动位置（逻辑像素，内容顶部到视口顶部的距离）。
    pub scroll: f32,
    /// 上一帧绘制的区域（窗口坐标）；不显示时为空矩形。
    pub area: Rect,
    /// 上一帧排出的内容总高度（逻辑像素），滚动时按它限制范围。
    pub height: f32,
}

impl Default for PdfPreview {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        PdfPreview {
            doc: None,
            generation: 0,
            requests: None,
            wanted: Arc::default(),
            requested: HashSet::new(),
            sender,
            receiver,
            pages: None,
            rendered: HashMap::new(),
            failed: HashMap::new(),
            scroll: 0.0,
            area: Rect::default(),
            height: 0.0,
        }
    }
}

impl PdfPreview {
    /// 结束渲染线程，释放各页的位图，回到没有文档的状态。
    fn reset(&mut self) {
        self.doc = None;
        self.requests = None;
        self.generation += 1;
        self.requested.clear();
        if let Ok(mut wanted) = self.wanted.lock() {
            wanted.clear();
        }
        self.pages = None;
        self.rendered.clear();
        self.failed.clear();
        self.scroll = 0.0;
        self.height = 0.0;
    }

    /// 最大滚动位置（逻辑像素）。
    pub(crate) fn max_scroll(&self) -> f32 {
        (self.height - self.area.h).max(0.0)
    }

    /// 是否还在等渲染线程的结果（测试中等待用）。
    #[cfg(all(test, any(target_os = "macos", windows)))]
    pub(crate) fn is_busy(&self) -> bool {
        self.requests.is_some() && (self.pages.is_none() || !self.requested.is_empty())
    }
}

impl App {
    /// 当前标签是可以预览的 PDF 时，按需要打开渲染线程；不再显示 PDF 时结束线程、释放位图。
    ///
    /// 每次绘制前调用。
    pub(crate) fn advance_pdf(&mut self) {
        let active = self
            .editor
            .active()
            .filter(|d| d.unsupported == Some(Unsupported::Pdf))
            .map(|d| (d.id, d.revision, d.path.clone()));
        let pdf = &mut self.pdf;
        let Some((id, revision, Some(path))) = active else {
            if pdf.doc.is_some() {
                pdf.reset();
            }
            return;
        };
        if pdf.doc == Some((id, revision)) {
            return;
        }
        // 同一文档的版本改变（磁盘上的文件被修改）时保留滚动位置。
        let scroll = (pdf.doc.map(|(d, _)| d) == Some(id)).then_some(pdf.scroll);
        pdf.reset();
        pdf.scroll = scroll.unwrap_or(0.0);
        pdf.doc = Some((id, revision));
        let (requests, queue) = mpsc::channel::<Request>();
        pdf.requests = Some(requests);
        let (generation, sender, wanted, proxy) = (
            pdf.generation,
            pdf.sender.clone(),
            Arc::clone(&pdf.wanted),
            self.proxy.clone(),
        );
        let notify = move || {
            if let Some(proxy) = &proxy {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        };
        // 系统的 PDF 对象只能在打开它的线程上使用：文档在线程里打开，请求通道关闭时线程结束。
        let spawned = std::thread::Builder::new()
            .name("winc-pdf".to_owned())
            .spawn(move || {
                let document = match Document::open(&path) {
                    Ok(document) => document,
                    Err(reason) => {
                        let _ = sender.send(Done::Opened(generation, Err(reason)));
                        notify();
                        return;
                    }
                };
                let pages = document.pages().to_vec();
                if sender.send(Done::Opened(generation, Ok(pages))).is_err() {
                    return;
                }
                notify();
                for request in queue {
                    let key = (request.page, request.width);
                    if !wanted.lock().is_ok_and(|w| w.contains(&key)) {
                        continue;
                    }
                    let pixels = document
                        .render(request.page, request.width, request.height)
                        .map(Arc::new);
                    let done = Done::Rendered(generation, request.page, request.width, pixels);
                    if sender.send(done).is_err() {
                        return;
                    }
                    notify();
                }
            });
        if spawned.is_err() {
            pdf.requests = None;
            pdf.pages = Some(Err(winc_i18n::t!("preview-pdf-render-failed").to_owned()));
        }
    }

    /// 收下渲染线程送回的结果。
    pub(crate) fn poll_pdf(&mut self) {
        let mut changed = false;
        while let Ok(done) = self.pdf.receiver.try_recv() {
            let pdf = &mut self.pdf;
            match done {
                Done::Opened(generation, opened) if generation == pdf.generation => {
                    pdf.pages = Some(opened.map(Arc::from));
                    if pdf.pages.as_ref().is_some_and(Result::is_err) {
                        pdf.requests = None;
                    }
                    changed = true;
                }
                Done::Rendered(generation, page, width, pixels) if generation == pdf.generation => {
                    pdf.requested.remove(&(page, width));
                    match pixels {
                        Ok(pixels) => {
                            pdf.failed.remove(&(page, width));
                            pdf.rendered.insert(page, pixels);
                        }
                        Err(reason) => {
                            pdf.failed.insert((page, width), reason);
                        }
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            self.request_redraw();
        }
    }

    /// 这一帧需要的页：（页号，位图宽度，位图高度），单位为物理像素。
    ///
    /// 不在其中的页从缓存中移除，也不再渲染；缺少的页交给渲染线程。绘制时调用。
    pub(crate) fn request_pdf_pages(&mut self, pages: &[(usize, u32, u32)]) {
        let pdf = &mut self.pdf;
        let keys: HashSet<(usize, u32)> = pages.iter().map(|&(p, w, _)| (p, w)).collect();
        pdf.rendered
            .retain(|&page, pixels| keys.contains(&(page, pixels.width)));
        pdf.requested.retain(|key| keys.contains(key));
        pdf.failed.retain(|key, _| keys.contains(key));
        if let Ok(mut wanted) = pdf.wanted.lock() {
            wanted.clone_from(&keys);
        }
        let Some(requests) = pdf.requests.as_ref() else {
            return;
        };
        for &(page, width, height) in pages {
            let key = (page, width);
            let ready = pdf.rendered.get(&page).is_some_and(|p| p.width == width);
            // 失败过的页不再请求，否则每一帧都会重新渲染。
            if ready || pdf.requested.contains(&key) || pdf.failed.contains_key(&key) {
                continue;
            }
            if requests
                .send(Request {
                    page,
                    width,
                    height,
                })
                .is_ok()
            {
                pdf.requested.insert(key);
            }
        }
    }

    /// 处理 PDF 预览中的按键：上下方向键滚动一小段，`PageUp`、`PageDown`、空格滚动一屏，
    /// `Home`、`End` 到开头、结尾。返回是否处理；当前标签不是 PDF 时返回 `false`。
    pub(in crate::app) fn pdf_key(&mut self, key: &Key) -> bool {
        if self.pdf.doc.is_none() {
            return false;
        }
        // 翻一屏时与上一屏重叠一段，翻页后仍能看到上一屏末尾的内容。
        let page = (self.pdf.area.h - ARROW_STEP * 2.0).max(ARROW_STEP);
        match key {
            Key::Named(NamedKey::ArrowUp) => self.scroll_pdf(ARROW_STEP),
            Key::Named(NamedKey::ArrowDown) => self.scroll_pdf(-ARROW_STEP),
            Key::Named(NamedKey::PageUp) => self.scroll_pdf(page),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => self.scroll_pdf(-page),
            Key::Named(NamedKey::Home) => self.scroll_pdf(f32::MAX),
            Key::Named(NamedKey::End) => self.scroll_pdf(-f32::MAX),
            _ => return false,
        }
        true
    }

    /// 把 PDF 预览滚动 `dy`（逻辑像素，向下滚动为负）。
    pub(crate) fn scroll_pdf(&mut self, dy: f32) {
        let pdf = &mut self.pdf;
        pdf.scroll = (pdf.scroll - dy).clamp(0.0, pdf.max_scroll());
        self.request_redraw();
    }

    /// 测试中模拟渲染线程送回第 `page` 页（请求宽度 `width`）渲染失败的结果。
    #[cfg(all(test, any(target_os = "macos", windows)))]
    pub(crate) fn fail_pdf_page(&mut self, page: usize, width: u32, reason: &str) {
        let done = Done::Rendered(self.pdf.generation, page, width, Err(reason.to_owned()));
        let _ = self.pdf.sender.send(done);
        self.poll_pdf();
    }

    /// 测试中等渲染线程完成打开和已发出的渲染请求，并收下结果。
    #[cfg(all(test, any(target_os = "macos", windows)))]
    pub(crate) fn wait_for_pdf(&mut self) {
        self.advance_pdf();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.pdf.is_busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.poll_pdf();
        }
    }
}
