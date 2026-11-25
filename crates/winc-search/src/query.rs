//! 搜索的条件：查询文本、匹配选项，以及「包含」「排除」两组路径规则。

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use ignore::overrides::{Override, OverrideBuilder};

use crate::matcher::Options;

/// 在多个文件中搜索的条件：查询文本、匹配选项和路径过滤规则。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// 查找的文本；`regex` 为 `true` 时是正则表达式。
    pub text: String,
    /// 是否区分大小写。
    pub case_sensitive: bool,
    /// 全字匹配：匹配的两端不能紧挨着字母、数字或下划线。
    pub whole_word: bool,
    /// 是否按正则匹配。
    pub regex: bool,
    /// 只搜索匹配的文件：逗号分隔的 glob，例如 `src, *.rs`；为空时不限制。
    pub include: String,
    /// 是否也搜索被 `.gitignore`、`.ignore` 和全局规则忽略的文件。
    pub include_ignored: bool,
    /// 不搜索匹配的文件：逗号分隔的 glob。
    pub exclude: String,
    /// 每处匹配都含有的一段文字，大小写规则与 `case_sensitive` 相同，用于跳过不含它的内容；
    /// 为空时没有这项信息。
    pub required: String,
}

impl Query {
    /// 返回匹配选项。
    pub fn options(&self) -> Options {
        Options {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
        }
    }
}

/// 「包含」或「排除」中的 glob 有误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobError {
    /// 出错的是「排除」（否则是「包含」）。
    pub exclude: bool,
    /// 错误的原因。
    pub message: String,
}

impl fmt::Display for GlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GlobError {}

/// 「包含」「排除」两组路径规则。
///
/// 「排除」在遍历时剪掉整个目录；「包含」只筛选遍历出的文件，不会让被忽略的文件重新出现。
///
/// 规则编译一次，搜索的各个线程共用。
#[derive(Debug, Clone)]
pub struct Filter {
    include: Arc<Override>,
    exclude: Arc<Override>,
}

impl Filter {
    /// 把「包含」「排除」两组 glob 编译为规则，模式相对于 `root`。
    ///
    /// 各模式以逗号分隔：不含 `/` 的模式在任意一级目录生效（`src` 匹配所有名为 `src` 的目录和
    /// 文件）；以 `./` 或 `/` 开头的模式只从根目录起算；模式指向目录时，目录内的所有文件都匹配。
    /// Windows 上 `\` 与 `/` 同样分隔路径（那里的 glob 不用 `\` 转义）。
    ///
    /// # Errors
    ///
    /// 某个 glob 语法有误时返回 [`GlobError`]，写明出错的是哪一组。
    pub fn new(root: &Path, include: &str, exclude: &str) -> Result<Self, GlobError> {
        Ok(Self {
            include: Arc::new(globs(root, include, false)?),
            exclude: Arc::new(globs(root, exclude, true)?),
        })
    }

    /// 遍历目录时用的规则：剪掉「排除」的目录和文件。
    pub fn pruning(&self) -> Override {
        Override::clone(&self.exclude)
    }

    /// 遍历出的文件 `path` 是否在「包含」之内；「包含」为空时都在。
    pub fn includes(&self, path: &Path) -> bool {
        !self.include.matched(path, false).is_ignore()
    }
}

/// 把一组逗号分隔的 glob 编译为规则；`exclude` 为真时匹配的路径被排除，否则只留下匹配的路径。
///
/// # Errors
///
/// 某个 glob 语法有误时返回 [`GlobError`]。
fn globs(root: &Path, text: &str, exclude: bool) -> Result<Override, GlobError> {
    let error = |e: ignore::Error| GlobError {
        exclude,
        message: e.to_string(),
    };
    let prefix = if exclude { "!" } else { "" };
    let mut builder = OverrideBuilder::new(root);
    for pattern in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let pattern = separators(pattern);
        let pattern = pattern.trim_end_matches('/');
        let anchored = pattern.starts_with("./") || pattern.starts_with('/');
        let body = pattern.trim_start_matches("./").trim_start_matches('/');
        let base = if anchored || body.contains('/') {
            format!("/{body}")
        } else {
            format!("**/{body}")
        };
        for glob in [base.clone(), format!("{base}/**")] {
            builder.add(&format!("{prefix}{glob}")).map_err(error)?;
        }
    }
    builder.build().map_err(error)
}

/// 把 Windows 上用 `\` 分隔的路径规则换成 `/` 分隔；其他系统上 `\` 是 glob 的转义符，原样返回。
fn separators(pattern: &str) -> std::borrow::Cow<'_, str> {
    if cfg!(windows) {
        pattern.replace('\\', "/").into()
    } else {
        pattern.into()
    }
}
