//! 位图：`<image>` 元素引用的内嵌图片，以及 SVG 文件所在目录里的图片。

use super::*;

impl<'a, 'input> Parser<'a, 'input> {
    /// 处理 `<image>`：内嵌（`data:` URL）或同目录的位图。
    ///
    /// 解码后的位图一直保留到绘图释放。一张 SVG 中各不相同的位图合计不超过单张图片解码的上限
    /// （`crate::image::PIXELS_MAX` 像素，按 RGBA 计），超出的不绘制并记入 `missing`。
    pub(super) fn image(
        &mut self,
        node: Node<'a, 'input>,
        props: &Props,
        ctm: Affine,
        out: &mut Vec<Item>,
    ) {
        if !props.visible {
            return;
        }
        let Some(link) = node
            .attribute((XLINK, "href"))
            .or_else(|| node.attribute("href"))
            .map(str::trim)
        else {
            return;
        };
        let pixels = match self.images.get(link) {
            Some(cached) => cached.clone(),
            None => {
                let budget = crate::image::PIXELS_MAX * 4;
                let loaded = self.load_image(link).filter(|p| {
                    let bytes = self.image_bytes + p.rgba.len() as u64;
                    if bytes > budget {
                        self.miss(winc_i18n::t!("preview-svg-missing-images-too-large"));
                        return false;
                    }
                    self.image_bytes = bytes;
                    true
                });
                let loaded = loaded.map(Arc::new);
                self.images.insert(link, loaded.clone());
                loaded
            }
        };
        let Some(pixels) = pixels else {
            return;
        };
        let (vw, vh) = self.viewport;
        let (iw, ih) = (pixels.width as f32, pixels.height as f32);
        let x = len_attr(node, "x", vw);
        let y = len_attr(node, "y", vh);
        let given = |name: &str, reference: f32| {
            node.attribute(name)
                .filter(|v| v.trim() != "auto")
                .and_then(|v| length(v, reference))
        };
        // 只写了一边时，另一边按图片的宽高比推算；两边都没写时取图片原始大小。
        let (w, h) = match (given("width", vw), given("height", vh)) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * ih / iw),
            (None, Some(h)) => (h * iw / ih, h),
            (None, None) => (iw, ih),
        };
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let aspect = aspect(node.attribute("preserveAspectRatio"));
        let place = fit([0.0, 0.0, iw, ih], aspect, w, h).then(Affine::translate(x, y));
        self.shapes += 1;
        let item = Item::Image(ImageItem {
            pixels,
            transform: place.then(ctm),
            pixelated: props.pixelated,
        });
        // `slice`（铺满）时超出的部分裁掉。
        let clip = aspect.slice.then(|| rect_clip(x, y, w, h, ctm));
        push_group(
            out,
            (1.0, filter::Blend::Normal),
            clip,
            None,
            None,
            vec![item],
        );
    }

    /// 读取 `<image>` 引用的位图：`data:` URL、`file://` 路径，或相对 `base` 的文件路径。
    ///
    /// 网络上的图片、嵌入的 SVG 图片、找不到或无法解码的图片不绘制，并记入 `missing`。
    fn load_image(&mut self, link: &str) -> Option<Pixels> {
        let bytes = if let Some(data) = link.strip_prefix("data:") {
            let (meta, payload) = data.split_once(',')?;
            if meta.ends_with(";base64") {
                base64(payload)?
            } else {
                percent_decode(payload)
            }
        } else if link.contains("://") && !link.starts_with("file://") {
            self.miss(winc_i18n::t!("preview-svg-missing-remote-images"));
            return None;
        } else {
            let path = link.strip_prefix("file://").unwrap_or(link);
            let path = percent_decode(path);
            let path = Path::new(std::str::from_utf8(&path).ok()?);
            let full = if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.base.as_ref()?.join(path)
            };
            // 只读取普通文件，且大小不超过 `crate::image::FILE_MAX`：`href="/dev/zero"` 或管道的
            // 读取没有尽头。
            let file = winc_backend::fs::metadata(&full)
                .ok()
                .filter(|m| m.is_file() && m.len <= crate::image::FILE_MAX);
            match file.and_then(|_| winc_backend::fs::read(&full).ok()) {
                Some(bytes) => bytes,
                None => {
                    self.miss(winc_i18n::t!("preview-svg-missing-image-not-found"));
                    return None;
                }
            }
        };
        if bytes.trim_ascii_start().starts_with(b"<") {
            self.miss(winc_i18n::t!("preview-svg-missing-nested-svg"));
            return None;
        }
        match crate::image::decode_bytes(&bytes) {
            Ok(pixels) => Some(pixels),
            Err(_) => {
                self.miss(winc_i18n::t!("preview-svg-missing-image-undecodable"));
                None
            }
        }
    }
}
