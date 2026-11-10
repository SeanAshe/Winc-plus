//! 图片与 SVG 预览。
//!
//! 位图解码和 SVG 光栅化都在后台线程中进行：标签先打开，任务完成后再显示。只处理当前标签，
//! 因此恢复上次打开的多个图片标签时不会同时解码全部图片。

use std::collections::HashSet;
use std::sync::{Arc, OnceLock, mpsc};

use winc_editor::DocumentId;
use winc_editor::{PictureState, PreviewMode};
use winc_preview::image::{self, Picture, Pixels};
use winc_preview::svg;

use crate::app::{App, UserEvent};

/// 后台完成的加载任务：文档、任务依据的文档版本号（`revision`）、加载结果。
type Done = (DocumentId, u64, Result<Picture, String>);
/// 后台按指定大小绘制完成的 SVG：文档、源图片、绘制得到的像素。
type Rendered = (DocumentId, Arc<Picture>, Pixels);

/// 图片与 SVG 的后台任务状态：任务结果的通道，以及进行中的任务记录。
pub(crate) struct ImageJobs {
    sender: mpsc::Sender<Done>,
    receiver: mpsc::Receiver<Done>,
    /// 已提交后台、尚未返回的加载任务，键为（文档，文档版本号）。
    pending: HashSet<(DocumentId, u64)>,
    rendered_sender: mpsc::Sender<Rendered>,
    rendered: mpsc::Receiver<Rendered>,
    /// 正在后台绘制的 SVG：（文档，图片的 `Arc` 指针地址，目标大小（物理像素））。
    ///
    /// 完成前不提交新的绘制（见 [`App::render_picture`]）。
    drawing: Option<(DocumentId, usize, (u32, u32))>,
    split_sender: mpsc::Sender<Done>,
    split_done: mpsc::Receiver<Done>,
    /// 最近显示过的两个标签：当前标签和它之前的那一个。
    ///
    /// 其余标签解码好的图在后台释放；保留前一个，两张图之间来回切换时不必重新解码。
    shown: [Option<DocumentId>; 2],
}

/// 分栏预览（[`PreviewMode::Split`]）右边那张图：跟着文字实时重画。
///
/// 同一时刻只在后台解析一版：打字快过解析时，上一版回来以后直接解析最新的，中间的版本跳过，
/// 不用定时器。写到一半、解析不了的时候还显示上一张画好的图，下面一行写出错在哪里，不会每
/// 输入一个字符就显示一页报错。
#[derive(Debug, Default)]
pub(crate) struct SplitPreview {
    /// 预览的是哪个文档；当前标签换了时从头开始。
    pub doc: Option<DocumentId>,
    /// 最近一次解析成功的图。
    pub picture: Option<Arc<Picture>>,
    /// 最新解析过的那一版解析不了时的原因。
    pub error: Option<String>,
    /// 最新解析过的文档版本（`Document::revision`），成功与否都算。
    checked: Option<u64>,
    /// 正在后台解析的版本。
    pending: Option<u64>,
}

impl Default for ImageJobs {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (rendered_sender, rendered) = mpsc::channel();
        let (split_sender, split_done) = mpsc::channel();
        Self {
            sender,
            receiver,
            pending: HashSet::new(),
            rendered_sender,
            rendered,
            drawing: None,
            split_sender,
            split_done,
            shown: [None; 2],
        }
    }
}

/// 使用系统字体绘制 SVG 中的文字。
///
/// 字体文件在第一次用到时才映射，之后常驻内存，所有 SVG 共用。
struct SystemGlyphs(winc_render::OutlineFonts);

impl svg::Glyphs for SystemGlyphs {
    fn glyph(&self, families: &[String], weight: u16, italic: bool, c: char) -> Option<svg::Glyph> {
        self.0
            .glyph(families, weight, italic, c)
            .map(|g| svg::Glyph {
                outline: g.outline,
                advance: g.advance,
            })
    }

    fn metrics(&self, families: &[String], weight: u16, italic: bool) -> svg::FontMetrics {
        let m = self.0.metrics(families, weight, italic);
        svg::FontMetrics {
            ascent: m.ascent,
            descent: m.descent,
            x_height: m.x_height,
        }
    }
}

/// 返回所有 SVG 共用的系统字体字形来源，首次调用时创建。
pub(crate) fn system_glyphs() -> Arc<dyn svg::Glyphs> {
    static GLYPHS: OnceLock<Arc<SystemGlyphs>> = OnceLock::new();
    GLYPHS
        .get_or_init(|| Arc::new(SystemGlyphs(winc_render::OutlineFonts::new())))
        .clone()
}

impl App {
    /// 当前标签换了时，释放既不是当前标签、也不是前一个标签的文档里解码好的图。
    fn release_hidden_pictures(&mut self) {
        let active = self.editor.active().map(|d| d.id);
        let shown = &mut self.images.shown;
        if active.is_none() || shown[0] == active {
            return;
        }
        *shown = [active, shown[0]];
        let keep = *shown;
        for doc in &mut self.editor.documents {
            if !keep.contains(&Some(doc.id)) {
                doc.release_picture();
                doc.release_table();
            }
        }
    }

    /// 当前标签的图片尚未加载时，提交后台加载任务；每次绘制前调用。
    pub(crate) fn advance_images(&mut self) {
        self.advance_split();
        self.advance_markdown();
        self.advance_pdf();
        self.release_hidden_pictures();
        self.advance_tables();
        let Some(doc) = self.editor.active() else {
            return;
        };
        if !matches!(doc.picture, Some(PictureState::Loading)) {
            return;
        }
        let key = (doc.id, doc.revision);
        if self.images.pending.contains(&key) {
            return;
        }
        // 位图按路径读取；SVG 使用文档当前文本渲染，因此包含未保存的修改。
        let job: Box<dyn FnOnce() -> Result<Picture, String> + Send> = if doc.unsupported.is_none()
        {
            let Some(rope) = doc.text().rope() else {
                return;
            };
            let text = rope.to_string();
            let base = doc
                .path
                .as_deref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf());
            Box::new(move || svg::load(&text, base.as_deref(), Some(system_glyphs())))
        } else {
            let Some(path) = doc.path.clone() else {
                return;
            };
            Box::new(move || image::load(&path))
        };
        self.images.pending.insert(key);
        let (sender, proxy) = (self.images.sender.clone(), self.proxy.clone());
        winc_editor::background::spawn(move || {
            let result = job();
            if sender.send((key.0, key.1, result)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 当前标签分栏预览、右边的图落后于文字时，在后台解析最新的文字（没保存的修改也算）。
    fn advance_split(&mut self) {
        let Some(doc) = self
            .editor
            .active()
            .filter(|d| d.preview_mode() == PreviewMode::Split)
            .filter(|d| d.preview_kind() == Some(winc_editor::PreviewKind::Svg))
        else {
            return;
        };
        let (id, revision) = (doc.id, doc.revision);
        if self.split.doc != Some(id) {
            self.split = SplitPreview {
                doc: Some(id),
                ..SplitPreview::default()
            };
        }
        if self.split.checked == Some(revision) || self.split.pending.is_some() {
            return;
        }
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let text = rope.to_string();
        let base = doc.path.as_deref().and_then(|p| p.parent()).map(Into::into);
        self.split.pending = Some(revision);
        let (sender, proxy) = (self.images.split_sender.clone(), self.proxy.clone());
        winc_editor::background::spawn(move || {
            let base: Option<std::path::PathBuf> = base;
            let result = svg::load(&text, base.as_deref(), Some(system_glyphs()));
            if sender.send((id, revision, result)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 收下分栏预览解析好的一版；文字又改了时下一帧再解析最新的版本。
    fn accept_split(&mut self, (id, revision, result): Done) {
        let split = &mut self.split;
        // 不是正在等的那一版（换过标签，或从预览切过来时已经接手了现成的图）就忽略。
        if split.doc != Some(id) || split.pending != Some(revision) {
            return;
        }
        split.pending = None;
        split.checked = Some(revision);
        match result {
            Ok(picture) => {
                split.picture = Some(Arc::new(picture));
                split.error = None;
            }
            Err(reason) => split.error = Some(reason),
        }
        self.request_redraw();
    }

    /// 提交后台任务，将 SVG `picture` 绘制为 `size`（物理像素，宽、高）的位图。
    ///
    /// 拖动窗口边缘时 `size` 每帧都在变化。同一时刻只在后台绘制一张：已有绘制在进行时不提交，
    /// 那一张完成后界面重绘，届时仍不符合的尺寸再提交。否则每帧一张，过时的绘制挤在共用的
    /// 线程池里，排在 ⌘P、全局搜索之前。
    pub(crate) fn render_picture(
        &mut self,
        id: DocumentId,
        picture: &Arc<Picture>,
        size: (u32, u32),
    ) {
        let key = (id, Arc::as_ptr(picture) as usize, size);
        if self.images.drawing.is_some() {
            return;
        }
        self.images.drawing = Some(key);
        let (sender, proxy, picture) = (
            self.images.rendered_sender.clone(),
            self.proxy.clone(),
            Arc::clone(picture),
        );
        winc_editor::background::spawn(move || {
            let pixels = picture.render(size.0, size.1);
            if sender.send((id, picture, pixels)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 接收后台完成的结果：加载好的图片放入对应文档，绘制好的 SVG 放入缩放缓存。
    pub(crate) fn poll_images(&mut self) {
        while let Ok(done) = self.images.receiver.try_recv() {
            self.accept_image(done);
        }
        while let Ok(done) = self.images.split_done.try_recv() {
            self.accept_split(done);
        }
        while let Ok(rendered) = self.images.rendered.try_recv() {
            self.accept_rendered(rendered);
        }
        self.poll_markdown();
        self.poll_pdf();
        self.poll_tables();
        self.poll_wallpaper();
    }

    fn accept_rendered(&mut self, (id, picture, pixels): Rendered) {
        let key = (
            id,
            Arc::as_ptr(&picture) as usize,
            (pixels.width, pixels.height),
        );
        if self.images.drawing == Some(key) {
            self.images.drawing = None;
        }
        // 文档或图片已经切换时丢弃结果。大小与最新窗口尺寸不符的结果仍先使用，
        // 其清晰度高于拉伸旧图；下一帧会按新尺寸重新提交绘制。
        let split = self
            .split
            .picture
            .as_ref()
            .filter(|_| self.split.doc == Some(id));
        let current = self
            .editor
            .active()
            .filter(|d| d.id == id)
            .and_then(|d| d.picture().or(split));
        if current.is_some_and(|p| Arc::ptr_eq(p, &picture)) {
            self.picture_cache = Some(PictureCache {
                doc: id,
                picture,
                pixels,
                exact: true,
            });
            self.request_redraw();
        }
    }

    fn accept_image(&mut self, (id, revision, result): Done) {
        self.images.pending.remove(&(id, revision));
        if let Some(index) = self.editor.index_of(id)
            && self.editor.documents[index].set_picture(revision, result)
        {
            self.request_redraw();
        }
    }

    /// 切换源代码与预览（SVG、Markdown）；当前文件不能预览时在状态栏提示。
    pub(in crate::app) fn toggle_preview(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let mode = match doc.preview_mode() {
            PreviewMode::Preview => PreviewMode::Source,
            PreviewMode::Source | PreviewMode::Split => PreviewMode::Preview,
        };
        self.choose_preview_mode(mode);
    }

    /// 用户选择以 `mode` 显示当前标签（标签栏右端的三段切换、切换预览的命令）：切换成功时
    /// 记住该方式，之后打开的能预览的文件按它显示，下次启动沿用。
    pub(in crate::app) fn choose_preview_mode(&mut self, mode: PreviewMode) {
        self.set_preview_mode(mode);
        if self.editor.active().map(|d| d.preview_mode()) != Some(mode) {
            return;
        }
        self.editor.preview_default = mode;
        let saved = match mode {
            PreviewMode::Source => None,
            PreviewMode::Split => Some("split"),
            PreviewMode::Preview => Some("preview"),
        };
        self.update_state(|state| state.preview_mode = saved.map(str::to_owned));
    }

    /// 按 `state.json` 记下的方式设定之后打开的能预览的文件的显示方式（启动、新开窗口时）。
    pub(crate) fn apply_preview_state(&mut self) {
        self.editor.preview_default = match self.state.preview_mode.as_deref() {
            Some("split") => PreviewMode::Split,
            Some("preview") => PreviewMode::Preview,
            _ => PreviewMode::Source,
        };
    }

    /// 当前标签换成 `mode` 显示（标签栏右端的三段切换）。
    pub(in crate::app) fn set_preview_mode(&mut self, mode: PreviewMode) {
        // 换了显示方式，键盘先回到文字（只看预览时本来就对着预览）。
        self.markdown.focused = false;
        let top = self.leave_markdown_mode(mode);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        // 分栏和预览解析的是同一段文字：版本没变时把已经解析好的图交过去，不再解析一遍，
        // 只按新的大小重画。
        let (id, revision) = (doc.id, doc.revision);
        let split = &self.split;
        let from_split = split
            .picture
            .clone()
            .filter(|_| split.doc == Some(id) && split.checked == Some(revision))
            .filter(|_| split.error.is_none());
        let from_preview = doc.picture().cloned();
        if !doc.set_preview_mode(mode) {
            self.flash(winc_i18n::t!("preview-mode-unsupported"));
            return;
        }
        match mode {
            PreviewMode::Preview => {
                if let Some(picture) = from_split {
                    doc.picture = Some(PictureState::Ready(picture));
                }
            }
            PreviewMode::Split => {
                if let Some(picture) = from_preview
                    && !(self.split.doc == Some(id) && self.split.checked == Some(revision))
                {
                    self.split = SplitPreview {
                        doc: Some(id),
                        picture: Some(picture),
                        checked: Some(revision),
                        ..SplitPreview::default()
                    };
                }
            }
            PreviewMode::Source => {}
        }
        self.relayout();
        // 从只看预览换回来：文字滚到预览顶上的那一处，不回到换走之前的位置。
        if let Some(line) = top {
            self.scroll_text_to(line);
        }
        self.request_redraw();
    }
}

/// 缩放后、按物理像素一一对应绘制的图片缓存：记录属于哪个文档的哪张图，以及缩放到多大。
///
/// 窗口大小改变时才重新缩放。
pub(crate) struct PictureCache {
    /// 图片所属的文档。
    pub(crate) doc: winc_editor::DocumentId,
    /// 缩放前的原图。
    pub(crate) picture: std::sync::Arc<winc_preview::image::Picture>,
    /// 缩放后的像素。
    pub(crate) pixels: winc_preview::image::Pixels,
    /// 是否按当前大小绘制；为 `false` 时先沿用上一次的缩放结果（SVG 正在后台按新大小重绘）。
    pub(crate) exact: bool,
}

#[cfg(test)]
mod tests;
