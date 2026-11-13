//! Markdown 预览中的图片：后台读取与解码、按显示大小绘制，以及释放屏幕之外的图片数据。

use super::*;

impl App {
    /// 请求图片 `path`（文件中写的地址为 `url`）：尚未读取时交给后台读取，网址的图片先下载。
    pub(crate) fn request_markdown_image(&mut self, path: &Path, url: &str) {
        if self.preview.markdown.images.contains_key(path) {
            return;
        }
        let md = &mut self.preview.markdown;
        md.images.insert(path.to_path_buf(), ImageSlot::Loading);
        md.urls.insert(path.to_path_buf(), url.to_owned());
        let (sender, proxy, path) = (
            md.sender.clone(),
            self.link.proxy.clone(),
            path.to_path_buf(),
        );
        let read = self.markdown_image_reader(&path, url);
        let load = move || {
            if sender
                .send(Done::Loaded(path, read().map(Arc::new)))
                .is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        };
        // 网络图片由 `curl` 下载，最长等待 `REMOTE_TIMEOUT`；
        // 在单独的线程上等待，不占用共用的线程池。
        if is_remote(url) {
            winc_editor::background::spawn_blocking(load);
        } else {
            winc_editor::background::spawn(load);
        }
    }

    /// 返回在后台线程上读取并解码图片 `path`（文件中写的地址为 `url`）的函数：文档内嵌的图片
    /// 取内嵌的数据，网址的图片先下载，其余从磁盘读取。
    pub(super) fn markdown_image_reader(
        &self,
        path: &Path,
        url: &str,
    ) -> impl FnOnce() -> Result<Picture, String> + Send + 'static {
        let path = path.to_path_buf();
        let remote = is_remote(url).then(|| url.to_owned());
        let embedded = self.preview.markdown.embedded.get(&path).cloned();
        move || match (&remote, embedded) {
            (_, Some(bytes)) if image::is_svg(&path) => std::str::from_utf8(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|text| svg::load(text, None, Some(system_glyphs()))),
            (_, Some(bytes)) => image::load_bytes(&bytes),
            (Some(url), None) => download(url, &path).and_then(|()| load_downloaded(&path)),
            (None, None) if image::is_svg(&path) => winc_backend::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| svg::load(&text, path.parent(), Some(system_glyphs()))),
            (None, None) => image::load(&path),
        }
    }

    /// 请求把图片 `path` 按 `size`（物理像素）绘制：已有该尺寸的像素时不提交。
    ///
    /// 每张图同一时刻只在后台绘制一个尺寸：拖动改变窗口大小时每帧尺寸都不同，正在绘制时不提交，
    /// 当前尺寸完成后界面重绘，届时仍缺的尺寸再提交。解码后的图片已经释放时
    /// （见 [`Self::release_markdown_images`]）先重新读取。
    pub(crate) fn request_markdown_pixels(&mut self, path: &Path, size: (u32, u32)) {
        let Some(ImageSlot::Ready {
            picture,
            pixels,
            drawing,
            ..
        }) = self.preview.markdown.images.get_mut(path)
        else {
            return;
        };
        if pixels.iter().any(|p| (p.width, p.height) == size) || !drawing.is_empty() {
            return;
        }
        drawing.push(size);
        let picture = picture.clone();
        let url = self.preview.markdown.urls.get(path).cloned();
        let reread = picture.is_none().then(|| {
            let url = url.as_deref().unwrap_or_default();
            (is_remote(url), self.markdown_image_reader(path, url))
        });
        let (sender, proxy, path) = (
            self.preview.markdown.sender.clone(),
            self.link.proxy.clone(),
            path.to_path_buf(),
        );
        let remote = reread.as_ref().is_some_and(|(remote, _)| *remote);
        let draw = move || {
            let picture = match (picture, reread) {
                (Some(picture), _) => Ok(picture),
                (None, Some((_, read))) => read().map(Arc::new),
                (None, None) => Err(String::new()),
            };
            let done = match picture {
                Ok(picture) => Done::Drawn(path, Arc::new(picture.render(size.0, size.1))),
                Err(_) => Done::Undrawn(path, size),
            };
            if sender.send(done).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        };
        // 重新读取网址的图片可能要再次下载，与首次读取相同，在单独的线程上等待。
        if remote {
            winc_editor::background::spawn_blocking(draw);
        } else {
            winc_editor::background::spawn(draw);
        }
    }

    /// 释放这一帧没有用到的图片数据：`shown` 是这一帧绘制范围内的图片，`wanted` 是其中还缺
    /// 显示大小的像素、已请求绘制的图片。
    ///
    /// 排版只需要图片的宽高，绘制只需要显示大小的像素。不在绘制范围内的图片释放像素和解码后
    /// 的图片；在绘制范围内、像素已经齐全的图片只释放解码后的图片。占用的内存因此只取决于
    /// 屏幕上的图片，与文档中图片的数量无关；图片再次进入绘制范围时重新读取。
    pub(crate) fn release_markdown_images(&mut self, shown: &[PathBuf], wanted: &[PathBuf]) {
        for (path, slot) in &mut self.preview.markdown.images {
            let ImageSlot::Ready {
                picture,
                pixels,
                drawing,
                ..
            } = slot
            else {
                continue;
            };
            // 正在后台绘制的图片保持不变：绘制的结果仍然要采用。
            if !drawing.is_empty() {
                continue;
            }
            if !shown.contains(path) {
                pixels.clear();
                *picture = None;
            } else if !wanted.contains(path) {
                *picture = None;
            }
        }
    }
}
