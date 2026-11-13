//! 在后台保存：大文件按片段整个写出需要数秒，远端主机上的文件要等待网络，在界面线程上写
//! 会阻塞事件循环。
//!
//! 大文件：
//! 发起时复制片段表（[`LargeText::try_clone`]，与原文件共用，不复制内容），后台按这份片段表写出
//! 并替换原文件，界面线程上可以继续编辑。写完时期间没有再改就改为引用刚写好的文件，旧文件占的
//! 磁盘空间随之释放；期间又改过则继续引用原文件，原文件已被改名替换，但打开着的文件句柄仍指向
//! 原来的内容。
//!
//! 远端主机上的普通文档：发起时复制文本（与文档共用，不复制内容），后台先确认远端的文件自上次
//! 读写后没有被其他程序修改，再写出。界面线程上不能等待远端的应答，这项检查因此放在这里。

use super::*;

use winc_i18n::t;

/// 一次要在后台做的保存，由 [`Document::begin_save`] 创建。
pub struct SaveJob {
    /// 发起时文档的路径。
    path: PathBuf,
    /// 写给哪个文件：路径是符号链接时取链接指向的文件。
    target: PathBuf,
    /// 发起时的内容。
    text: Snapshot,
    /// 发起时的撤销版本，即 [`winc_core::History::seal`] 的返回值。
    version: u64,
    /// 发起时的文档版本号（`revision`）。
    revision: u64,
}

/// 发起保存时的内容。
enum Snapshot {
    /// 大文件的片段表副本，以及是否在文件开头写入 UTF-8 BOM。
    Large { text: Box<LargeText>, bom: bool },
    /// 普通文档的文本、编码，以及上次读写时文件的修改时间和大小。
    Rope {
        rope: Rope,
        encoding: Encoding,
        known: (Option<Stamp>, u64),
    },
}

/// 后台保存的结果，交给 [`Document::finish_save`]。
pub struct Saved {
    path: PathBuf,
    version: u64,
    revision: u64,
    result: io::Result<Written>,
    /// 文件已被其他程序修改而没有写入时，文件当前的修改时间和大小。
    changed: Option<(Option<Stamp>, u64)>,
}

/// 写好的文件。
struct Written {
    /// 重新打开的刚写好的文件；打不开时为 `None`，继续引用原来的原文件。
    fresh: Option<LargeText>,
    mtime: Option<Stamp>,
    len: u64,
}

impl SaveJob {
    /// 写出并改名替换原文件（在后台线程上调用）。大文件随后打开写好的文件并建好行索引，耗时
    /// 和文件大小成正比。
    pub fn run(self) -> Saved {
        let mut changed = None;
        let result = match &self.text {
            // 片段引用原文件的字节范围，不用原地写入（会先截断原文件）。
            Snapshot::Large { text, bom } => write_atomic(&self.target, *bom, false, |out| {
                text.write_to(out)
            })
            .map(|written| Written {
                fresh: winc_backend::fs::open(&self.target, 0)
                    .and_then(|opened| LargeText::open(opened.into_handle()))
                    .ok(),
                mtime: written.modified,
                len: written.len,
            }),
            Snapshot::Rope {
                rope,
                encoding,
                known,
            } => {
                let found = winc_backend::fs::metadata(&self.target).ok();
                let found = found.map(|metadata| (metadata.modified, metadata.len));
                if found.is_some_and(|found| found != *known) {
                    changed = found;
                    Err(io::Error::other(t!("editor-original-changed")))
                } else {
                    write_rope(&self.target, rope, *encoding).map(|written| Written {
                        fresh: None,
                        mtime: written.modified,
                        len: written.len,
                    })
                }
            }
        };
        Saved {
            path: self.path,
            version: self.version,
            revision: self.revision,
            result,
            changed,
        }
    }
}

impl Document {
    /// 发起后台保存；本机上的普通文档返回 `None`。
    ///
    /// 返回的 [`SaveJob`] 在后台线程上调用 `SaveJob::run`，写完交给 [`Document::finish_save`]；
    /// 返回 `None` 时调用方走 [`Document::save`]。
    ///
    /// # Errors
    ///
    /// 与 [`Document::save`] 能发现的错误相同：没有路径、只读、原文件在打开后被其他程序修改。
    /// 复制文件句柄失败时同样返回错误。写入时的错误在 [`Document::finish_save`] 里返回。
    pub fn begin_save(&mut self) -> io::Result<Option<SaveJob>> {
        let Some(path) = &self.path else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                t!("editor-file-no-path"),
            ));
        };
        if let Some(reason) = self.readonly_reason() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, reason));
        }
        let text = match &self.text {
            Content::Rope(_) if !winc_backend::is_remote(path) => return Ok(None),
            Content::Rope(rope) => Snapshot::Rope {
                rope: rope.clone(),
                encoding: self.encoding,
                known: (self.disk_mtime, self.disk_len),
            },
            Content::Large(large) if large.original_changed() => {
                return Err(io::Error::other(t!("editor-original-changed")));
            }
            Content::Large(large) => Snapshot::Large {
                text: Box::new(large.try_clone()?),
                bom: self.encoding == Encoding::Utf8Bom,
            },
        };
        Ok(Some(SaveJob {
            path: path.clone(),
            target: winc_config::files::canonicalize(path).unwrap_or_else(|_| path.clone()),
            text,
            version: self.history.seal(),
            revision: self.revision,
        }))
    }

    /// 收下后台保存的结果：把发起时的版本记为已保存，记下磁盘文件的当前状态，清除冲突标记。
    ///
    /// # Errors
    ///
    /// 写入失败时返回 `std::io::Error`，文档不变。保存期间文档的路径改变（重命名、移动）时同样
    /// 返回错误且文档不变：写入的是原来的路径，新路径上的文件没有保存。
    pub fn finish_save(&mut self, saved: Saved) -> io::Result<()> {
        // 文件已被其他程序修改：记下它当前的状态并标记冲突，由用户决定保留哪一边。
        if let Some(found) = saved.changed
            && self.path.as_deref() == Some(saved.path.as_path())
        {
            (self.disk_mtime, self.disk_len) = found;
            self.conflict = true;
        }
        let written = saved.result?;
        if self.path.as_deref() != Some(saved.path.as_path()) {
            return Err(io::Error::other(t!("editor-moved-while-saving")));
        }
        if self.revision == saved.revision {
            // 期间没改过：和 `save` 一样封口，之后的键入另起一个撤销单元。
            self.history.mark_saved();
            if let Some(fresh) = written.fresh {
                self.text = Content::Large(Box::new(fresh));
            }
        } else {
            self.history.mark_saved_version(saved.version);
        }
        self.disk_mtime = written.mtime;
        self.disk_len = written.len;
        self.conflict = false;
        Ok(())
    }
}
