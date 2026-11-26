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

    /// 模拟后台已为文档 `id` 的格式化选好工具 `plan`、结果尚未处理，此时标签可能已经关闭。
    #[cfg_attr(not(unix), allow(dead_code))] // 只有 Unix 上的格式化测试用到
    pub(in crate::app) fn plan_formatting(&mut self, id: DocumentId, plan: Plan) {
        let doc = self.editor.document(id).unwrap();
        let launch = Launch {
            task: Task::Format,
            id,
            save: false,
            range: None,
            sent: doc.format_source().unwrap(),
            line_ending: doc.line_ending,
            remote: false,
            root: None,
            project: None,
        };
        self.format.planning.push((id, false));
        self.format.planned.0.send((launch, Ok(plan))).unwrap();
    }

    /// 返回正在后台格式化的文件数。
    pub(in crate::app) fn formatting_count(&self) -> usize {
        self.format.running.len()
    }
}
