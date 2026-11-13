//! 文件树里在后台进行的复制（拖入、粘贴）：超过一定时间还没有结束时在右上角显示进度，
//! 可以取消。

use std::sync::Arc;
use std::time::{Duration, Instant};

use winc_editor::Copying;
use winc_i18n::t;

use super::*;

/// 复制开始后多久还没有结束才显示进度（800 ms）：很快结束的复制不显示提示。
///
/// 取值与常见编辑器在资源管理器中复制、上传文件时显示进度之前的等待一致。
const PROGRESS_DELAY: Duration = Duration::from_millis(800);

/// 进度上的数字每隔多久更新一次（1 秒）：更新更频繁时数字跳动，难以读出。
///
/// 取值与常见编辑器上传文件时进度文字的更新间隔一致。
const PROGRESS_REFRESH: Duration = Duration::from_secs(1);

/// 文件树里正在后台进行的各次复制。
#[derive(Default)]
pub struct Copies {
    /// 尚未结束的各次复制，按开始的先后。
    running: Vec<Arc<Copying>>,
    /// 下一次显示或更新进度的时刻；没有复制在进行时为 `None`。
    due: Option<Instant>,
}

// 用到它的测试只在 Unix 上编译。
#[cfg(all(test, unix))]
impl Copies {
    /// 把 `copying` 记为正在进行、已到显示进度的时刻的一次复制。
    pub(in crate::app) fn track(&mut self, copying: Arc<Copying>) {
        self.running.push(copying);
        self.due = Some(Instant::now());
    }
}

/// 把字节数 `bytes` 写成便于阅读的大小：不足 1 KB 时写字节数，其余取一位小数（1 KB 为 1024
/// 字节）。
fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

/// 复制 `copying` 的进度说明：正在复制的一项的名字和已复制的大小，单个文件时还有总大小。
fn progress_label(copying: &Copying) -> String {
    let (name, total) = copying.item();
    let done = size_label(copying.copied());
    match total {
        Some(total) => t!(
            "workspace-tree-copying-file",
            name = name,
            done = done,
            total = size_label(total)
        ),
        None => t!("workspace-tree-copying-folder", name = name, done = done),
    }
}

impl App {
    /// 在后台执行一次复制 `work`，完成后把它的结果经 `done` 包成事件送回界面线程；`remote` 表示
    /// 有一侧在远端主机上。没有事件循环（测试中）时就地执行，返回结果。
    ///
    /// 内容经网络传输时可能很久，不占用共用的线程池。
    pub(in crate::app) fn spawn_copy<R: Send + 'static>(
        &mut self,
        remote: bool,
        work: impl FnOnce(&Copying) -> R + Send + 'static,
        done: impl FnOnce(R, Arc<Copying>) -> UserEvent + Send + 'static,
    ) -> Option<R> {
        let copying = Arc::new(Copying::default());
        let Some(proxy) = self.link.proxy.clone() else {
            return Some(winc_backend::remote::allow_blocking(|| work(&copying)));
        };
        self.panels.tree.copies.running.push(Arc::clone(&copying));
        if self.panels.tree.copies.due.is_none() {
            self.panels.tree.copies.due = Some(Instant::now() + PROGRESS_DELAY);
        }
        let task = move || {
            let result = work(&copying);
            let _ = proxy.send_event(done(result, copying));
        };
        if remote {
            winc_editor::background::spawn_blocking(task);
        } else {
            winc_editor::background::spawn(task);
        }
        None
    }

    /// 复制 `copying` 结束：不再显示它的进度，返回它是否是被取消的。
    pub(in crate::app) fn copy_finished(&mut self, copying: &Arc<Copying>) -> bool {
        let copies = &mut self.panels.tree.copies;
        copies.running.retain(|other| !Arc::ptr_eq(other, copying));
        if copies.running.iter().all(|other| other.is_cancelled()) {
            copies.due = None;
            if self.notice.action == Some(NoticeAction::CancelCopy) {
                self.dismiss_notice();
            }
        }
        if copying.is_cancelled() {
            self.flash(t!("workspace-tree-copy-canceled"));
        }
        copying.is_cancelled()
    }

    /// 取消正在进行的各次复制：各自正在复制的一项停止，已复制完的各项保留。
    pub(in crate::app) fn cancel_copies(&mut self) {
        for copying in &self.panels.tree.copies.running {
            copying.cancel();
        }
        self.panels.tree.copies.due = None;
    }

    /// 到时刻时显示或更新复制的进度，返回下一次的时刻；没有复制在进行时返回 `None`。
    ///
    /// 显示最早开始的一次。右上角正显示着别的提示时不替换它，等它消失之后再显示。
    pub(in crate::app) fn copy_tick(&mut self, now: Instant) -> Option<Instant> {
        let due = self.panels.tree.copies.due?;
        if now < due {
            return Some(due);
        }
        let ours = self.notice.action == Some(NoticeAction::CancelCopy);
        let running =
            (self.panels.tree.copies.running.iter()).find(|copying| !copying.is_cancelled());
        if let Some(message) = running.map(|copying| progress_label(copying)) {
            if ours {
                // 只换文字：提示条留在原处，不重新播放入场动画。
                if self.notice.text.as_deref() != Some(message.as_str()) {
                    self.notice.text = Some(message);
                    self.request_redraw();
                }
            } else if self.notice.text.is_none() {
                self.notify_pinned(message, NoticeAction::CancelCopy);
            }
        }
        let next = now + PROGRESS_REFRESH;
        self.panels.tree.copies.due = Some(next);
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::size_label;

    #[test]
    fn sizes_are_written_with_one_decimal() {
        assert_eq!(size_label(0), "0 B");
        assert_eq!(size_label(1023), "1023 B");
        assert_eq!(size_label(1024), "1.0 KB");
        assert_eq!(size_label(66_000_000), "62.9 MB");
        assert_eq!(size_label(3 << 30), "3.0 GB");
    }
}
