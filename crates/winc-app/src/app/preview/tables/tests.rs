//! `tables` 的测试辅助：等待当前标签的表格读取完成，按正常路径收下结果。

use super::*;

impl App {
    /// 等当前标签的表格读好，按正常路径收下。
    pub(crate) fn wait_for_tables(&mut self) {
        let wait = std::time::Duration::from_secs(10);
        loop {
            self.advance_tables();
            if self.preview.tables.pending.is_empty() {
                break;
            }
            let done = self
                .preview
                .tables
                .receiver
                .recv_timeout(wait)
                .expect("表格一直没读好");
            // 放回通道，按正常路径收下。
            let _ = self.preview.tables.sender.send(done);
            self.poll_tables();
        }
        crate::app::snapshot::paint(self);
    }
}
