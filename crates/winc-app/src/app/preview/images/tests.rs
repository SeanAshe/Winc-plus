//! 图片预览的测试：等待后台解码和 SVG 绘制完成。

use super::*;

impl SplitPreview {
    /// 测试用：后台是否正在解析。
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

impl App {
    /// 测试用：等分栏预览把当前的文字解析完。
    pub(crate) fn wait_for_split(&mut self) {
        let wait = std::time::Duration::from_secs(10);
        self.advance_split();
        while self.split.pending.is_some() {
            let done = self
                .images
                .split_done
                .recv_timeout(wait)
                .expect("分栏预览超时");
            self.accept_split(done);
            self.advance_split();
        }
    }

    /// 阻塞等待后台任务全部完成（仅用于测试）。
    ///
    /// 先等待图片加载完成，再反复绘制一帧并等待该帧提交的 SVG 绘制完成，直到绘制不再提交新的
    /// 任务（低分辨率位图和可见的部分分两次绘制）。每一步最长等待 10 秒。
    ///
    /// # Panics
    ///
    /// 任一步等待超时时 panic。
    pub(crate) fn wait_for_images(&mut self) {
        let wait = std::time::Duration::from_secs(10);
        self.advance_images();
        while !self.images.pending.is_empty() {
            match self.images.receiver.recv_timeout(wait) {
                Ok(done) => self.accept_image(done),
                Err(_) => panic!("图片一直没解好"),
            }
        }
        crate::app::snapshot::paint(self);
        while self.images.drawing.is_some() {
            match self.images.rendered.recv_timeout(wait) {
                Ok(rendered) => self.accept_rendered(rendered),
                Err(_) => panic!("SVG 一直没画好"),
            }
            crate::app::snapshot::paint(self);
        }
    }
}
