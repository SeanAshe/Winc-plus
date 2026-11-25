//! 把文件整理成可以粘贴到对话中的提示词：每个文件一段，`File: 路径` 后跟按扩展名标注语言的
//! Markdown 代码块。
//!
//! 只收文本文件：二进制文件、无法无损解码的文件、单个或累计超过 [`MAX_PROMPT_BYTES`] 的文件跳过。
//! 解码规则与打开文件相同（[`winc_search::text::decode`]）；有未保存修改的文件用调用方给出的内容。

use std::io::Read;
use std::path::{Path, PathBuf};

use ropey::Rope;

/// 一次最多整理的文件数（个），取自调参：多选一个目录下的源文件时够用，选错时不在后台逐个读取
/// 整棵树。
pub const MAX_PROMPT_FILES: usize = 128;

/// 提示词的大小上限（字节，1 MiB）：常见对话界面单条消息能容纳的文字远小于此，超出部分粘贴后
/// 也会被截断，没有必要读取。
pub const MAX_PROMPT_BYTES: usize = 1 << 20;

/// 要整理进提示词的一个文件。
#[derive(Debug, Clone)]
pub struct PromptFile {
    /// 文件的路径。
    pub path: PathBuf,
    /// 写在 `File:` 之后的名字：相对于所在目录的路径。
    pub name: String,
    /// 有未保存修改时编辑器中的内容；为 `None` 时读取磁盘上的文件。
    pub unsaved: Option<Rope>,
}

/// 整理好的提示词。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prompt {
    /// 各文件的段落依次相连；没有可用的文件时为空。
    pub text: String,
    /// 跳过的文件数（个）。
    pub skipped: usize,
}

/// 按顺序把 `files` 整理成提示词，规则见模块说明。
///
/// 读取磁盘上的文件，耗时与文件的个数和大小成正比，应在后台线程调用；远端的文件还要等待连接。
pub fn prompt(files: Vec<PromptFile>) -> Prompt {
    let mut out = Prompt::default();
    for file in files {
        let room = MAX_PROMPT_BYTES - out.text.len();
        let text = match file.unsaved {
            Some(rope) => (rope.len_bytes() <= room).then(|| rope.to_string()),
            None => read_text(&file.path, room),
        };
        let block = text.map(|text| file_block(&file.name, language(&file.path), &text));
        match block {
            Some(block) if block.len() <= room => out.text.push_str(&block),
            _ => out.skipped += 1,
        }
    }
    out
}

/// 读取文本文件 `path` 并解码；不是普通文件（跟随符号链接）、超过 `limit` 字节、是二进制文件或
/// 无法无损解码时返回 `None`。
///
/// 读取量以 `limit` 为限：文件在查询大小之后变大也不会读入更多。
fn read_text(path: &Path, limit: usize) -> Option<String> {
    let mut file = winc_backend::fs::open(path, 0).ok()?;
    if !file.metadata.is_file() || file.metadata.len > limit as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    let mut reader = file.reader().ok()?.take(limit as u64 + 1);
    reader.read_to_end(&mut bytes).ok()?;
    if bytes.len() > limit || winc_search::looks_binary(&bytes) {
        return None;
    }
    let decoded = winc_search::text::decode(&bytes);
    (!decoded.lossy()).then(|| decoded.text.into_owned())
}

/// 代码块标注的语言：文件的扩展名，只含 ASCII 字母和数字时才写；否则为空。
fn language(path: &Path) -> &str {
    let extension = path.extension().and_then(|ext| ext.to_str());
    extension
        .filter(|ext| ext.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or("")
}

/// 返回一个文件的段落：`File: name`、代码块和一个空行。
///
/// 代码块的围栏比 `text` 中最长的一串反引号多一个（至少 3 个），文件中的代码块不会提前结束
/// 外层的代码块；`name` 中的换行写成 `\n`，不打断 `File:` 一行。
fn file_block(name: &str, language: &str, text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    let name = name.replace('\r', "\\r").replace('\n', "\\n");
    let newline = if text.is_empty() || text.ends_with(['\r', '\n']) {
        ""
    } else {
        "\n"
    };
    format!("File: {name}\n{fence}{language}\n{text}{newline}{fence}\n\n")
}

#[cfg(test)]
mod tests;
