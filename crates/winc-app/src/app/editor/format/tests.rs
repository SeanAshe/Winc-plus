//! 格式化的测试：后台结果的收取与保存前的步骤。

use super::*;

impl App {
    /// 等待后台格式化送回一个结果并处理，最长等待 20 秒；没有正在进行的格式化时立即返回。
    ///
    /// 测试中没有事件循环。
    pub(in crate::app) fn wait_formatting(&mut self) {
        if self.format.running.is_empty() {
            return;
        }
        if let Ok((id, result)) = self.format.receiver.recv_timeout(Duration::from_secs(20)) {
            self.formatting_done(id, result);
        }
    }

    /// 保存文档 `id` 时是否先格式化。
    pub(in crate::app) fn formats_on_save(&self, id: DocumentId) -> bool {
        self.on_save(Task::Format, id)
    }

    /// 返回正在后台格式化的文件数。
    pub(in crate::app) fn formatting_count(&self) -> usize {
        self.format.running.len()
    }
}
