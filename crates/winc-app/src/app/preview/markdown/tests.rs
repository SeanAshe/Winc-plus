//! `markdown` 的测试：等待后台解析、读图和绘制完成的辅助方法，以及下载的 SVG 徽章绘制文字。

use super::*;

impl MarkdownPreview {
    /// 测试用：后台还有没做完的解析、读图或画图。
    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
            || self.images.values().any(|slot| match slot {
                ImageSlot::Loading => true,
                ImageSlot::Ready { drawing, .. } => !drawing.is_empty(),
                ImageSlot::Failed => false,
            })
    }

    /// 测试用：等一个后台结果，最多等 10 秒。
    fn wait(&self) -> Option<Done> {
        self.receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .ok()
    }
}

impl App {
    /// 测试用：等后台正在做的解析、读图和绘制全部完成并收下结果（绘制一帧可能又提出新的
    /// 任务，调用方再次调用）。
    pub(crate) fn wait_for_markdown(&mut self) {
        self.advance_markdown();
        while self.preview.markdown.is_busy() {
            let done = self.preview.markdown.wait().expect("Markdown 预览超时");
            // 放回通道，按正常路径收下。
            let _ = self.preview.markdown.sender.send(done);
            self.poll_markdown();
        }
    }
}

#[test]
fn file_links_stay_local() {
    let doc = Path::new("/repo/docs/README.md");
    assert_eq!(
        local_target(doc, "file:///Applications/Calculator.app"),
        Some(PathBuf::from("/Applications/Calculator.app"))
    );
    assert_eq!(
        local_target(doc, "file://host/a%20b.md#top"),
        Some(PathBuf::from("/a b.md"))
    );
    assert_eq!(
        local_target(doc, "guide.md"),
        Some(PathBuf::from("/repo/docs/guide.md"))
    );
    for url in ["https://a.dev", "smb://host/share", "mailto:a@b.c", ""] {
        assert_eq!(local_target(doc, url), None, "{url}");
    }
}

#[test]
fn downloaded_svg_badges_draw_their_text() {
    // 与 shields.io 徽章相同的结构：深色底上写白字。
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="90" height="20"><rect width="90" height="20" fill="#555"/><text x="45" y="14" fill="#fff" font-family="Verdana,DejaVu Sans,sans-serif" font-size="11" text-anchor="middle">Website</text></svg>"##;
    let dir = std::env::temp_dir().join(format!("winc-badge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("badge");
    std::fs::write(&path, svg).unwrap();
    let picture = load_downloaded(&path).unwrap();
    let pixels = picture.render(90, 20);
    // 底色是 #555，有字的地方是接近白色的像素。
    let light = pixels
        .rgba
        .chunks(4)
        .filter(|p| p[0] > 200 && p[3] > 0)
        .count();
    assert!(light > 20, "{light}");
    let _ = std::fs::remove_dir_all(&dir);
}
