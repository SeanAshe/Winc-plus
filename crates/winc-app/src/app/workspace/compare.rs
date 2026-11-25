//! 比较文件树中选中的两个文件。
//!
//! 在并排差异标签中打开：左栏是先选中的文件（显示顺序靠前者），右栏是另一个文件。两侧均按
//! 磁盘上的内容比较且只读，不依赖 Git。

use std::path::Path;

use winc_editor::diff_view::{DiffSide, DiffView};
use winc_editor::encoding::decode_unknown;
use winc_editor::{LARGE_FILE, display_name, looks_binary};
use winc_i18n::t;

use super::*;

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
        let (old, new) = match (read_for_compare(left), read_for_compare(right)) {
            (Ok(old), Ok(new)) => (old, new),
            (Err(e), _) | (_, Err(e)) => {
                self.flash(e);
                return;
            }
        };
        let language = winc_syntax::detect(&display_name(right));
        let relative = |path: &Path| {
            self.relative_path(path)
                .unwrap_or_else(|| path.to_string_lossy().into_owned())
        };
        let view = DiffView::new(
            DiffSide::new(relative(left), &old, None, language),
            DiffSide::new(relative(right), &new, None, language),
        );
        if view.is_identical() {
            self.flash(t!(
                "workspace-compare-identical",
                left = display_name(left),
                right = display_name(right)
            ));
            return;
        }

        self.show_compare(&compare_title(left, right), view);
    }
}
