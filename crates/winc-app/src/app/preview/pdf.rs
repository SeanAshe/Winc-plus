//! PDF 预览的状态：渲染线程、各页大小、渲染好的页和滚动位置。
//!
//! 每个正在显示的 PDF 有一个渲染线程：文档在该线程中打开（系统的 PDF 对象不能跨线程使用），按
//! 请求逐页渲染。绘制时只请求视口内和视口附近的页（`App::request_pdf_pages`）：不再需要的页从
//! 缓存中移除，线程中排队但已不需要的请求跳过，内存只占这几页的位图。切换标签或文件被修改时
//! 结束线程。左侧缩略图栏中的页同样按需渲染，排在正文的页之后。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, mpsc};

use winc_editor::{DocumentId, Unsupported};
use winc_preview::image::Pixels;
use winc_preview::pdf::{Document, PageSize};
use winc_render::{Rect, Renderer};
use winit::keyboard::{Key, NamedKey};

use crate::app::animation::{Clock, PDF_MOTION};
use crate::app::{App, UserEvent};

/// 方向键每次滚动的距离（逻辑像素）：约为正文三行的高度。
const ARROW_STEP: f32 = 48.0;

/// 页面列四周的留白（逻辑像素），与 Markdown 预览的留白相同。
const PAD: f32 = 32.0;
/// 相邻两页之间的间距（逻辑像素），为页面列留白 [`PAD`] 的一半。
pub(crate) const GAP: f32 = 16.0;
/// 缩放倍数为 1 时页面的最大显示宽度（逻辑像素）：A4 页在该宽度下放大约 1.45 倍，10.5 磅的
/// 正文约为 15 逻辑像素，便于阅读；更宽的编辑区不再放大。
pub(crate) const MAX_PAGE_WIDTH: f32 = 860.0;
/// 可选的缩放倍数，1 为适应宽度。
const ZOOMS: [f32; 7] = [0.5, 0.67, 0.8, 1.0, 1.25, 1.5, 2.0];
/// 最小和最大的缩放倍数。
pub(crate) const ZOOM_RANGE: (f32, f32) = (0.5, 2.0);
/// 一页位图的像素数上限（约 68 MB）：放大后超过它的倍数不可选。
///
/// 取 2 倍屏上最宽的 A4 页放大到 2 倍时的大小（3440 × 4866）。
const MAX_PAGE_PIXELS: f64 = 17_000_000.0;
/// 触控板捏合累计到该比例时缩放一档；PDF 预览与图片预览共用。
pub(in crate::app::preview) const PINCH_STEP: f32 = 0.2;

/// 各页在内容中的位置（逻辑像素，相对内容左上角），以及内容的宽度和高度。
///
/// 页面都按最宽的一页缩放到同一比例：倍数 `zoom` 为 1 时最宽的页适应宽度 `width`，不超过
/// [`MAX_PAGE_WIDTH`]。放大后比 `width` 宽的页可以横向滚动。
pub(crate) fn page_rects(pages: &[PageSize], width: f32, zoom: f32) -> (Vec<Rect>, f32, f32) {
    let widest = pages.iter().map(|p| p.width).fold(0.0, f64::max).max(1.0);
    let shown = (width - PAD * 2.0).clamp(1.0, MAX_PAGE_WIDTH) * zoom;
    let content = width.max(shown + PAD * 2.0);
    let scale = f64::from(shown) / widest;
    let mut y = PAD;
    let rects = pages
        .iter()
        .map(|page| {
            let (w, h) = ((page.width * scale) as f32, (page.height * scale) as f32);
            let rect = Rect::new((content - w) / 2.0, y, w, h);
            y += h + GAP;
            rect
        })
        .collect();
    (rects, content, y - GAP + PAD)
}

/// 右下角工具栏上的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PdfTool {
    /// 显示或隐藏缩略图栏。
    Thumbnails,
    /// 缩小一档。
    ZoomOut,
    /// 当前倍数，点击后还原为适应宽度。
    ZoomReset,
    /// 放大一档。
    ZoomIn,
}

/// 缩放的过渡：从倍数 `from` 到 [`PdfPreview::zoom`]，期间第 `page` 页内相对位置 `at` 处
/// 保持在正文区域内的 `anchor`（逻辑像素，相对正文区域左上角）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ZoomMotion {
    pub(crate) from: f32,
    pub(crate) clock: Clock,
    pub(crate) page: usize,
    pub(crate) at: (f32, f32),
    pub(crate) anchor: (f32, f32),
}

/// 渲染线程的请求：页号（从 0 起）、位图大小（物理像素），以及是否为缩略图。
struct Request {
    page: usize,
    width: u32,
    height: u32,
    thumb: bool,
}

/// 渲染线程送回的结果，带有打开时的序号：序号与当前序号不同时说明文档已经更换，结果丢弃。
enum Done {
    /// 打开的结果：各页大小，或失败的原因。
    Opened(u64, Result<Vec<PageSize>, String>),
    /// 渲染完的一页：页号、请求的位图宽度、是否为缩略图，以及位图或失败的原因。
    Rendered(u64, usize, u32, bool, Result<Arc<Pixels>, String>),
}

/// 当前标签的 PDF 预览。
pub(crate) struct PdfPreview {
    /// 上一帧绘制的区域（窗口坐标）；不显示时为空矩形。
    pub area: Rect,
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
    /// 上一帧排出的内容总高度（逻辑像素），滚动时按它限制范围。
    pub height: f32,
    /// 缩放倍数（见 `ZOOMS`）；换文档时保留，重启后恢复为 1。
    pub zoom: f32,
    /// 横向滚动位置和上一帧排出的内容宽度（逻辑像素）；页面比区域宽时可以横向滚动。
    pub scroll_x: f32,
    pub width: f32,
    /// 触控板捏合累计的缩放比例，见 `PINCH_STEP`。
    pinch: f32,
    /// 进行中的缩放过渡。
    pub(crate) zoom_motion: Option<ZoomMotion>,
    /// 缩略图栏正在滑入（真）或滑出，及其时钟。
    pub(crate) strip_motion: Option<(bool, Clock)>,
    /// 上一帧缩略图栏的区域（窗口坐标）；不显示时为空矩形。
    pub strip: Rect,
    /// 渲染好的缩略图：页号到位图，只保留缩略图栏当前需要的页。
    pub thumbs: HashMap<usize, Arc<Pixels>>,
    /// 缩略图栏的滚动位置和内容总高度（逻辑像素）。
    pub strip_scroll: f32,
    pub strip_height: f32,
    /// 上一帧的当前页（视口中线所在的页）；改变时缩略图栏滚动到该页。
    pub current: usize,
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
            zoom: 1.0,
            scroll_x: 0.0,
            width: 0.0,
            pinch: 0.0,
            zoom_motion: None,
            strip_motion: None,
            strip: Rect::default(),
            thumbs: HashMap::new(),
            strip_scroll: 0.0,
            strip_height: 0.0,
            current: 0,
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
        self.scroll_x = 0.0;
        self.pinch = 0.0;
        self.zoom_motion = None;
        self.thumbs.clear();
        self.strip_scroll = 0.0;
        self.strip_height = 0.0;
        self.current = 0;
    }

    /// 最大滚动位置（逻辑像素）。
    pub(crate) fn max_scroll(&self) -> f32 {
        (self.height - self.area.h).max(0.0)
    }

    /// 正文所在的区域：预览区域去掉缩略图栏。
    fn body(&self) -> Rect {
        let (area, left) = (self.area, self.strip.w);
        Rect::new(area.x + left, area.y, area.w - left, area.h)
    }

    /// 排出各页在倍数 `zoom` 下的位置，记下内容的宽度和高度，并限制滚动范围；`width` 为正文
    /// 区域的宽度。返回各页的位置，以及最终倍数与本帧倍数之比（过渡期间不为 1）。
    ///
    /// 缩放过渡期间倍数逐帧变化，锚点处的内容保持不动。绘制时调用。
    pub(crate) fn lay_out(&mut self, pages: &[PageSize], width: f32) -> (Vec<Rect>, f32) {
        let motion = self.zoom_motion;
        let zoom = motion.map_or(self.zoom, |m| m.clock.between(m.from, self.zoom));
        let (rects, content, height) = page_rects(pages, width, zoom);
        (self.width, self.height) = (content, height);
        if let Some(m) = motion {
            self.hold(&rects, m.page, m.at, m.anchor);
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        self.scroll_x = self.scroll_x.clamp(0.0, (content - width).max(0.0));
        (rects, self.zoom / zoom)
    }

    /// 滚动到第 `page` 页内相对位置 `at` 处位于 `anchor`（相对正文区域左上角）的位置。
    fn hold(&mut self, rects: &[Rect], page: usize, at: (f32, f32), anchor: (f32, f32)) {
        if let Some(page) = rects.get(page) {
            self.scroll = page.y + at.1 * page.h - anchor.1;
            self.scroll_x = page.x + at.0 * page.w - anchor.0;
        }
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
                let mut pending: VecDeque<Request> = VecDeque::new();
                loop {
                    if pending.is_empty() {
                        match queue.recv() {
                            Ok(request) => pending.push_back(request),
                            Err(_) => return,
                        }
                    }
                    pending.extend(queue.try_iter());
                    // 正文的页先于缩略图渲染。
                    let next = pending.iter().position(|r| !r.thumb).unwrap_or(0);
                    let Some(request) = pending.remove(next) else {
                        continue;
                    };
                    let key = (request.page, request.width);
                    if !wanted.lock().is_ok_and(|w| w.contains(&key)) {
                        continue;
                    }
                    let pixels = document
                        .render(request.page, request.width, request.height)
                        .map(Arc::new);
                    let (page, width, thumb) = (request.page, request.width, request.thumb);
                    let done = Done::Rendered(generation, page, width, thumb, pixels);
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
                Done::Rendered(generation, page, width, thumb, pixels)
                    if generation == pdf.generation =>
                {
                    pdf.requested.remove(&(page, width));
                    match pixels {
                        Ok(pixels) => {
                            pdf.failed.remove(&(page, width));
                            let cache = if thumb {
                                &mut pdf.thumbs
                            } else {
                                &mut pdf.rendered
                            };
                            cache.insert(page, pixels);
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

    /// 这一帧需要的页和缩略图：（页号，位图宽度，位图高度），单位为物理像素。
    ///
    /// 不在其中的从缓存中移除，也不再渲染；缺少的交给渲染线程。绘制时调用。
    pub(crate) fn request_pdf_pages(
        &mut self,
        pages: &[(usize, u32, u32)],
        thumbs: &[(usize, u32, u32)],
    ) {
        let pdf = &mut self.pdf;
        let key = |&(page, width, _): &(usize, u32, u32)| (page, width);
        let small: HashSet<(usize, u32)> = thumbs.iter().map(key).collect();
        pdf.thumbs
            .retain(|&page, pixels| small.contains(&(page, pixels.width)));
        let keys: HashSet<(usize, u32)> = pages.iter().map(key).collect();
        // 尺寸不符的旧位图留到新的渲染完成：缩放后先缩放显示旧位图，不出现空白。
        pdf.rendered
            .retain(|page, _| pages.iter().any(|(wanted, ..)| wanted == page));
        let keys: HashSet<(usize, u32)> = keys.union(&small).copied().collect();
        pdf.requested.retain(|key| keys.contains(key));
        pdf.failed.retain(|key, _| keys.contains(key));
        if let Ok(mut wanted) = pdf.wanted.lock() {
            wanted.clone_from(&keys);
        }
        let Some(requests) = pdf.requests.as_ref() else {
            return;
        };
        let all = (pages.iter().map(|page| (page, false))).chain(thumbs.iter().map(|t| (t, true)));
        for (&(page, width, height), thumb) in all {
            let key = (page, width);
            let cache = if thumb { &pdf.thumbs } else { &pdf.rendered };
            let ready = cache.get(&page).is_some_and(|p| p.width == width);
            // 失败过的页不再请求，否则每一帧都会重新渲染。
            if ready || pdf.requested.contains(&key) || pdf.failed.contains_key(&key) {
                continue;
            }
            if requests
                .send(Request {
                    page,
                    width,
                    height,
                    thumb,
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
        self.scroll_pdf_by(0.0, dy);
    }

    /// 把 PDF 预览横向滚动 `dx`、竖直滚动 `dy`（逻辑像素，向右、向下滚动为负）。
    pub(crate) fn scroll_pdf_by(&mut self, dx: f32, dy: f32) {
        let pdf = &mut self.pdf;
        pdf.scroll = (pdf.scroll - dy).clamp(0.0, pdf.max_scroll());
        let across = (pdf.width - pdf.body().w).max(0.0);
        pdf.scroll_x = (pdf.scroll_x - dx).clamp(0.0, across);
        self.request_redraw();
    }

    /// 鼠标是否位于 PDF 的正文上（不含缩略图栏）。
    pub(in crate::app) fn on_pdf_body(&self) -> bool {
        let (x, y) = self.mouse;
        self.pdf.doc.is_some() && self.pdf.body().contains(x, y)
    }

    /// 把 PDF 放大（`up` 为真）、缩小一档，或还原为适应宽度（`None`），返回当前是否在预览 PDF。
    ///
    /// `anchor`（窗口坐标）处的内容保持在原位，为 `None` 时取正文区域的中心。位图超过
    /// `MAX_PAGE_PIXELS` 的倍数不可选。
    pub(in crate::app) fn zoom_pdf(
        &mut self,
        up: Option<bool>,
        anchor: Option<(f32, f32)>,
    ) -> bool {
        let Some(Ok(pages)) = self.pdf.pages.clone() else {
            return self.pdf.doc.is_some();
        };
        let body = self.pdf.body();
        let scale = self.renderer.as_ref().map_or(1.0, Renderer::scale_factor);
        let old = self.pdf.zoom;
        let fits = |zoom: &f32| {
            let (rects, ..) = page_rects(&pages, body.w, *zoom);
            let pixels = |r: &Rect| f64::from(r.w * scale) * f64::from(r.h * scale);
            *zoom <= 1.0 || rects.iter().map(pixels).fold(0.0, f64::max) <= MAX_PAGE_PIXELS
        };
        let next = match up {
            Some(true) => ZOOMS.into_iter().find(|zoom| *zoom > old).filter(fits),
            Some(false) => ZOOMS.into_iter().rev().find(|zoom| *zoom < old),
            None => Some(1.0),
        };
        let zoom = next.unwrap_or(old);
        if zoom != old {
            // 记下锚点所在的页和它在页内的相对位置，缩放后（过渡期间逐帧）把同一处放回锚点。
            let (ax, ay) = anchor.unwrap_or((body.x + body.w / 2.0, body.y + body.h / 2.0));
            let anchor = (ax - body.x, ay - body.y);
            let (before, ..) = self.pdf.lay_out(&pages, body.w);
            let (cx, cy) = (self.pdf.scroll_x + anchor.0, self.pdf.scroll + anchor.1);
            let page = before.iter().rposition(|r| r.y <= cy).unwrap_or(0);
            if let Some(r) = before.get(page) {
                let at = ((cx - r.x) / r.w.max(1.0), (cy - r.y) / r.h.max(1.0));
                let from = (self.pdf.zoom_motion).map_or(old, |m| m.clock.between(m.from, old));
                self.pdf.zoom = zoom;
                // 有过渡时由每帧的排版推进；没有时立即到位。
                self.pdf.zoom_motion = self.clock(PDF_MOTION).map(|clock| ZoomMotion {
                    from,
                    clock,
                    page,
                    at,
                    anchor,
                });
                if self.pdf.zoom_motion.is_none() {
                    let (after, ..) = page_rects(&pages, body.w, zoom);
                    self.pdf.hold(&after, page, at, anchor);
                    self.pdf.lay_out(&pages, body.w);
                }
            }
        }
        let percent = (self.pdf.zoom * 100.0).round().to_string();
        self.flash(winc_i18n::t!("preview-pdf-zoom", zoom = percent));
        self.request_redraw();
        true
    }

    /// 处理触控板捏合 `delta`（放大为正）：鼠标在 PDF 的正文上时，累计满一档缩放一次。
    pub(crate) fn pdf_pinch(&mut self, delta: f32) {
        if !self.on_pdf_body() {
            return;
        }
        self.pdf.pinch += delta;
        if self.pdf.pinch.abs() >= PINCH_STEP {
            let up = self.pdf.pinch > 0.0;
            self.pdf.pinch = 0.0;
            self.zoom_pdf(Some(up), Some(self.mouse));
        }
    }

    /// 把缩略图栏滚动 `dy`（逻辑像素，向下滚动为负）。
    pub(crate) fn scroll_pdf_strip(&mut self, dy: f32) {
        let pdf = &mut self.pdf;
        let max = (pdf.strip_height - pdf.strip.h).max(0.0);
        pdf.strip_scroll = (pdf.strip_scroll - dy).clamp(0.0, max);
        self.request_redraw();
    }

    /// 把 PDF 预览滚动到位置 `to`（逻辑像素）；点击缩略图时调用。
    pub(in crate::app) fn scroll_pdf_to(&mut self, to: f32) {
        self.pdf.scroll = to.clamp(0.0, self.pdf.max_scroll());
        self.request_redraw();
    }

    /// 显示或隐藏缩略图栏（带滑入、滑出的过渡），并记入状态文件。
    pub(in crate::app) fn toggle_pdf_thumbnails(&mut self) {
        self.update_state(|state| state.pdf_thumbnails_hidden = !state.pdf_thumbnails_hidden);
        let shown = !self.state.pdf_thumbnails_hidden;
        self.pdf.strip_motion = self.clock(PDF_MOTION).map(|clock| (shown, clock));
        self.request_redraw();
    }

    /// 处理点击右下角工具栏上的按钮 `tool`。
    pub(in crate::app) fn press_pdf_tool(&mut self, tool: PdfTool) {
        match tool {
            PdfTool::Thumbnails => self.toggle_pdf_thumbnails(),
            PdfTool::ZoomOut => drop(self.zoom_pdf(Some(false), None)),
            PdfTool::ZoomReset => drop(self.zoom_pdf(None, None)),
            PdfTool::ZoomIn => drop(self.zoom_pdf(Some(true), None)),
        }
    }

    /// 测试中模拟渲染线程送回第 `page` 页（请求宽度 `width`）渲染失败的结果。
    #[cfg(all(test, any(target_os = "macos", windows)))]
    pub(crate) fn fail_pdf_page(&mut self, page: usize, width: u32, reason: &str) {
        let failed = Err(reason.to_owned());
        let done = Done::Rendered(self.pdf.generation, page, width, false, failed);
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
