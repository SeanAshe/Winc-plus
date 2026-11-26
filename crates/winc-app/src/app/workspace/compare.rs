//! 比较两个文件：文件树中选中的两个文件，或命令行上 `--diff` 给出的文件。
//!
//! 在并排差异标签中打开：左栏是先选中的文件（显示顺序靠前者）或命令行上的第一个文件，右栏是
//! 另一个文件。两侧均按磁盘上的内容比较且只读，不依赖 Git。

use std::path::Path;

use winc_editor::diff_view::{DiffSide, DiffView};
use winc_editor::encoding::decode_unknown;
use winc_editor::{LARGE_FILE, display_name, looks_binary};
use winc_i18n::t;

use super::*;
use crate::app::NoticeKind;
use crate::cli::Diff;

/// 返回 `path` 的文本内容用于比较；文件过大、是二进制文件或读取失败时返回提示文字。
fn read_for_compare(path: &Path) -> Result<String, String> {
    let name = display_name(path);
    // 先看大小：过大的文件不读进内存。
    if winc_backend::fs::metadata(path).is_ok_and(|m| m.len > LARGE_FILE) {
        return Err(t!("workspace-compare-too-large", name = name));
    }
    let bytes = winc_backend::fs::read(path).map_err(|e| {
        t!(
            "workspace-compare-read-failed",
            name = &*name,
            error = e.to_string()
        )
    })?;
    if bytes.len() as u64 > LARGE_FILE {
        return Err(t!("workspace-compare-too-large", name = name));
    }
    if looks_binary(&bytes) {
        return Err(t!("workspace-compare-binary", name = name));
    }
    Ok(decode_unknown(&bytes).0)
}

/// 返回比较两个文件时标签的标题，形式为「左 ↔ 右」。
pub(crate) fn compare_title(left: &Path, right: &Path) -> String {
    format!("{} ↔ {}", display_name(left), display_name(right))
}

impl App {
    /// 按磁盘上的内容比较文件 `left` 与 `right`，在并排差异标签中打开。
    ///
    /// 内容相同或读取失败时只显示提示，不打开标签。
    pub(in crate::app) fn compare_files(&mut self, left: &Path, right: &Path) {
        match self.compare_view(left, right) {
            Ok(view) if view.is_identical() => self.flash(identical(left, right)),
            Ok(view) => _ = self.show_compare(&compare_title(left, right), view),
            Err(e) => self.flash(e),
        }
    }

    /// 打开命令行上要比较的各对文件 `diffs`，每对一个并排差异标签。
    ///
    /// 命令行上明确要求比较：内容相同时同样打开标签，另作提示；读取失败时说明原因，不打开该对的
    /// 标签。
    pub(crate) fn open_diffs(&mut self, diffs: &[Diff]) {
        for Diff { left, right } in diffs {
            match self.compare_view(left, right) {
                Ok(view) => {
                    let same = view.is_identical();
                    self.show_compare(&compare_title(left, right), view);
                    if same {
                        self.flash(identical(left, right));
                    }
                }
                Err(e) => self.notify(NoticeKind::Warning, e),
            }
        }
    }

    /// 返回按磁盘上的内容比较 `left` 与 `right` 的差异；读取失败时返回提示文字。
    fn compare_view(&self, left: &Path, right: &Path) -> Result<DiffView, String> {
        let (old, new) = (read_for_compare(left)?, read_for_compare(right)?);
        let language = winc_syntax::detect(&display_name(right));
        let relative = |path: &Path| {
            self.relative_path(path)
                .unwrap_or_else(|| path.to_string_lossy().into_owned())
        };
        Ok(DiffView::new(
            DiffSide::new(relative(left), &old, None, language),
            DiffSide::new(relative(right), &new, None, language),
        ))
    }
}

/// 返回 `left` 与 `right` 内容相同的提示。
fn identical(left: &Path, right: &Path) -> String {
    t!(
        "workspace-compare-identical",
        left = display_name(left),
        right = display_name(right)
    )
    .to_owned()
}
