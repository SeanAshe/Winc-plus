//! 导出与导入设置：把用户级配置文件原样复制到其他位置，或用其他位置的文件替换配置文件。
//!
//! 只涉及配置文件（各项设置与快捷键）。状态文件记录本机的路径和对命令、仓库的许可，不导出也
//! 不导入：从文件导入许可会绕过执行命令前的确认。

use std::io;
use std::path::{Path, PathBuf};

use super::Config;
use crate::files::write_atomic;
use crate::json::Json;

/// [`import_config`] 的结果。
#[derive(Debug)]
pub struct Imported {
    /// 导入的文件中的配置。
    pub config: Config,
    /// 导入的文件中无法识别的项，均已忽略。
    pub problems: Vec<String>,
    /// 原配置文件另存的位置；原来没有配置文件或文件为空时为 `None`。
    pub backup: Option<PathBuf>,
}

/// 把配置文件 `config` 原样写入 `to`，保留注释；`config` 不存在时写入空对象，即全部为默认值。
///
/// # Errors
///
/// `config` 无法读取、`to` 无法写入时返回对应的 `io::Error`。
pub fn export_config(config: &Path, to: &Path) -> io::Result<()> {
    let text = match std::fs::read_to_string(config) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => "{}\n".to_owned(),
        Err(e) => return Err(e),
    };
    write_atomic(to, &text)
}

/// 用文件 `from` 替换配置文件 `config`：原样写入，保留注释；原文件先另存为同目录下的
/// `config.json.bak`。
///
/// 无法识别的项与读入配置时相同，忽略并列在 [`Imported::problems`] 中。
///
/// # Errors
///
/// `from` 无法读取时返回对应的 `io::Error`；内容不是顶层为对象的 JSON（允许注释）时返回
/// [`io::ErrorKind::InvalidData`]，配置文件保持不变；另存或写入失败时返回对应的 `io::Error`。
pub fn import_config(from: &Path, config: &Path) -> io::Result<Imported> {
    let bytes = std::fs::read(from)?;
    let invalid = || io::Error::from(io::ErrorKind::InvalidData);
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    if !Json::parse_jsonc(text).is_some_and(|(root, _)| root.as_object().is_some()) {
        return Err(invalid());
    }
    let backup = match std::fs::read(config) {
        Ok(old) if !old.trim_ascii().is_empty() => {
            let path = config.with_extension("json.bak");
            std::fs::write(&path, old)?;
            Some(path)
        }
        Ok(_) => None,
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    write_atomic(config, text)?;
    let (config, problems) = Config::parse(text);
    Ok(Imported {
        config,
        problems,
        backup,
    })
}

#[cfg(test)]
mod tests;
