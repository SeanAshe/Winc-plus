//! 读取进度：后台读文件时累计已读的字节数，界面线程据此显示读了多少。

use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 一次读取的进度，克隆后与读取的线程共享。
#[derive(Debug, Clone, Default)]
pub struct ReadProgress(Arc<Counts>);

#[derive(Debug, Default)]
struct Counts {
    read: AtomicU64,
    total: AtomicU64,
}

impl ReadProgress {
    /// 已读的比例（0 到 1）；还不知道文件大小时为 `None`。
    pub fn fraction(&self) -> Option<f32> {
        let total = self.0.total.load(Ordering::Relaxed);
        let read = self.0.read.load(Ordering::Relaxed);
        (total > 0).then(|| (read as f64 / total as f64).min(1.0) as f32)
    }

    /// 开始读一个 `total` 字节的文件。
    pub fn begin(&self, total: u64) {
        self.0.read.store(0, Ordering::Relaxed);
        self.0.total.store(total, Ordering::Relaxed);
    }

    /// 又读了 `bytes` 个字节。
    pub fn add(&self, bytes: u64) {
        self.0.read.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// 把读到的字节数计入 `progress` 的读取器。
pub(crate) struct Counted<'a, R> {
    pub inner: R,
    pub progress: Option<&'a ReadProgress>,
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if let Some(progress) = self.progress {
            progress.add(n as u64);
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_bytes_read_through_it() {
        let progress = ReadProgress::default();
        assert_eq!(progress.fraction(), None);
        progress.begin(8);
        let mut reader = Counted {
            inner: &b"abcdefgh"[..],
            progress: Some(&progress),
        };
        let mut buf = [0; 6];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(progress.fraction(), Some(0.75));
        let _ = reader.read(&mut buf).unwrap();
        assert_eq!(progress.fraction(), Some(1.0));
    }
}
