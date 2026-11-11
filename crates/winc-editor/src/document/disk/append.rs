//! 磁盘文件只在末尾追加了内容（例如日志持续写入）时，只读追加的部分，不整个重读。
//!
//! 读、比对和接上都在后台做（[`Reread::read`]）：发起时复制整个文档内容（rope 的复制只是加
//! 引用计数；大文件复制文件句柄和行索引，不复制内容），后台对着副本比对原内容、读追加的部分
//! 并接到副本末尾；界面线程上只把接好的副本换进文档（[`Document::apply_append`]）。

use super::*;

/// 只读追加部分要用的文档内容副本，见 [`Document::append_source`]。
pub(super) enum Append {
    /// 整个读进内存的文本，`disk_len` 是上次读写时文件的字节数，`bom` 是文件开头 BOM 的字节数。
    Rope { rope: Rope, disk_len: u64, bom: u64 },
    /// 大文件：在副本上把行索引向后扩展。
    Large(Box<LargeText>),
}

impl std::fmt::Debug for Append {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Append::Rope { .. } => "Append::Rope",
            Append::Large(_) => "Append::Large",
        })
    }
}

/// 后台读好的追加部分，交给 [`Document::apply_append`]。
///
/// `len`、`mtime` 是接上以后的文件大小（字节）和修改时间。
pub(super) enum Appended {
    /// 末尾已经接上追加内容的 rope，换掉原来那份。
    Rope {
        rope: Rope,
        len: u64,
        mtime: Option<SystemTime>,
    },
    /// 行索引已经扩展好的大文件，换掉原来那份。
    Large {
        text: Box<LargeText>,
        len: u64,
        mtime: Option<SystemTime>,
    },
}

impl Append {
    /// 读 `path` 末尾追加的部分（在后台线程上调用）。
    ///
    /// 不是只在末尾追加（原内容对不上、文件变短），或追加部分不是完整的 UTF-8 文字时返回
    /// `None`，由调用方整个重读。
    pub(super) fn read(self, path: &Path) -> Option<Appended> {
        match self {
            Append::Rope {
                rope,
                disk_len,
                bom,
            } => read_rope(path, rope, disk_len, bom),
            Append::Large(mut text) => {
                if text.grow().is_none() || text.has_overlong_lines() {
                    return None;
                }
                let metadata = fs::metadata(path).ok();
                Some(Appended::Large {
                    text,
                    len: metadata.as_ref().map_or(0, |m| m.len()),
                    mtime: metadata.and_then(|m| m.modified().ok()),
                })
            }
        }
    }
}

/// 读 `path` 中 `disk_len` 之后追加的文字，接到 `rope` 末尾。
///
/// 原内容不超过 `FULL_CHECK` 时整段与文件中同一位置的字节比对，更大时比对开头和末尾各 `TAIL`
/// 字节，判断原内容是否未变。以下情况返回 `None`：
///
/// - 原内容对不上，或追加部分含非法的 UTF-8 序列、像二进制内容；
/// - 追加量超过 `MAX_APPEND`，或新文件超过 [`LARGE_FILE`]。
fn read_rope(path: &Path, mut rope: Rope, disk_len: u64, bom: u64) -> Option<Appended> {
    /// 原内容较大时，开头和末尾各比对的字节数（字节，4 KiB）。
    const TAIL: u64 = 4 << 10;
    /// 一次追加的最大字节数（字节，16 MiB）。
    ///
    /// 追加部分整段读进内存后合成字符串，超过该值改由整个重读处理，使单次追加占用的内存有上限。
    /// 该上限取自调参：日志一类的文件每次追加量通常只有几 KB 到几百 KB。
    const MAX_APPEND: u64 = 16 << 20;
    /// 原内容不超过该值时整段比对（字节，1 MiB）。
    ///
    /// 整段比对要把原内容的这一段从 `rope` 和磁盘各读一遍，因此耗时随阈值线性增长。该阈值取自调参：
    /// 调大后整段比对覆盖的文件更大、判定更严格，检测的耗时随之增长；调小则更多文件只比对开头和
    /// 末尾，正文中途被改动同样长度的内容时可能判断不出来。1 MiB 是两者之间的折中。
    const FULL_CHECK: usize = 1 << 20;

    let mut file = fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    let (old, new) = (disk_len, metadata.len());
    if new <= old || new - old > MAX_APPEND || new > LARGE_FILE {
        return None;
    }

    // 校验原内容的末尾：rope 的最后几个字节应与文件中同一位置的字节相同。
    let content_len = old.saturating_sub(bom);
    if content_len != rope.len_bytes() as u64 {
        return None;
    }
    // 比对原内容：不大时整段比对；大了只比开头和末尾各一段，前面被原地改了同样长度的
    // 内容、又在末尾追加时，开头那段通常对不上，就整个重读。
    let content_len = content_len as usize;
    let ranges = if content_len <= FULL_CHECK {
        std::iter::once(0..content_len).collect::<Vec<_>>()
    } else {
        let tail = TAIL as usize;
        vec![0..tail, content_len - tail..content_len]
    };
    for range in ranges {
        // 两端对齐到字符边界，因为 rope 只能在字符边界上切片。
        let from = rope.char_to_byte(rope.byte_to_char(range.start));
        let to = rope.char_to_byte(rope.byte_to_char(range.end));
        let mut expected = Vec::with_capacity(to - from);
        for chunk in rope.byte_slice(from..to).chunks() {
            expected.extend_from_slice(chunk.as_bytes());
        }
        let mut actual = vec![0; to - from];
        if file.seek(io::SeekFrom::Start(bom + from as u64)).is_err()
            || file.read_exact(&mut actual).is_err()
            || actual != expected
        {
            return None;
        }
    }
    file.seek(io::SeekFrom::Start(old)).ok()?;

    let mut added = Vec::with_capacity((new - old) as usize);
    (&mut file).take(new - old).read_to_end(&mut added).ok()?;
    // 其他程序可能正在写入，最后一个字符只写入了部分字节：只追加前面完整的部分，其余留到下次。
    let complete = match std::str::from_utf8(&added) {
        Ok(_) => added.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => return None,
    };
    added.truncate(complete);
    let added = String::from_utf8(added).ok()?;
    if added.is_empty() || looks_binary(added.as_bytes()) {
        return None;
    }
    rope.insert(rope.len_chars(), &added);
    Some(Appended::Rope {
        rope,
        len: old + complete as u64,
        mtime: metadata.modified().ok(),
    })
}

impl Document {
    /// 能只读追加部分时返回所需的内容副本：编码是 UTF-8，大文件还要能复制文件句柄。
    pub(super) fn append_source(&self) -> Option<Append> {
        if !matches!(self.encoding, Encoding::Utf8 | Encoding::Utf8Bom) {
            return None;
        }
        match &self.text {
            Content::Rope(rope) => Some(Append::Rope {
                rope: rope.clone(),
                disk_len: self.disk_len,
                bom: if self.encoding == Encoding::Utf8Bom {
                    3
                } else {
                    0
                },
            }),
            Content::Large(large) => large.try_clone().ok().map(|l| Append::Large(Box::new(l))),
        }
    }

    /// 把后台读好的追加部分 `appended` 接到文档末尾。
    ///
    /// 撤销记录中的位置都在原内容范围内，追加到末尾不影响它们；当前内容就是磁盘上的内容。
    pub(super) fn apply_append(&mut self, appended: Appended) {
        let line = self.text.line_count() - 1;
        // 最后一行可能变长，其列索引不再可用。
        self.whole.forget();
        match appended {
            Appended::Rope { rope, len, mtime } => {
                self.text = Content::Rope(rope);
                (self.disk_len, self.disk_mtime) = (len, mtime);
                self.appended_rope(line);
            }
            // 大文件：原文留在磁盘上，换上行索引向后扩展过的那份。
            Appended::Large { text, len, mtime } => {
                self.text = Content::Large(text);
                (self.disk_len, self.disk_mtime) = (len, mtime);
                self.highlighter.invalidate_from(line);
                self.revision += 1;
            }
        }
        self.history.mark_saved();
    }

    /// rope 末尾接上了内容，原来的最后一行是第 `line` 行：重新高亮、更新折叠。
    fn appended_rope(&mut self, line: usize) {
        self.highlighter.invalidate_from(line);
        self.revision += 1;
        self.appended(line);
    }
}
