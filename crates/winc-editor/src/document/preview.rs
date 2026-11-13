//! 文档预览：图片、SVG、Markdown 与表格的呈现、编辑区与预览之间的切换，以及解码好的图片、
//! 读好的表格等派生状态。

use super::*;

impl Document {
    /// 返回编辑区显示的图；尚未解码完成或没有图时返回 `None`。
    pub fn picture(&self) -> Option<&Arc<Picture>> {
        match &self.picture {
            Some(PictureState::Ready(picture)) => Some(picture),
            _ => None,
        }
    }

    /// 是否在编辑区显示文本：不是图片、差异、日志、阅读标签或无法打开的文件，且 SVG 没有切换
    /// 到预览。
    ///
    /// 光标、查找、折叠等只对文本有意义。
    pub fn shows_text(&self) -> bool {
        self.unsupported.is_none()
            && self.picture.is_none()
            && (self.table.is_none() || self.split)
            && !self.rendered
            && self.diff.is_none()
            && self.log.is_none()
            && self.reading.is_none()
    }

    /// 是否能编辑文本：显示文本（见 [`Document::shows_text`]），或者是 Markdown 的预览（预览
    /// 上直接编辑，光标所在的块显示原文）。
    ///
    /// 编辑命令、粘贴、查找按它判断；只与编辑区的文本区有关的（折叠、粘滞滚动、逐行追溯）
    /// 仍按 [`Document::shows_text`]。
    pub fn edits_text(&self) -> bool {
        self.shows_text() || self.rendered
    }

    /// 是否可以切换到预览：对应磁盘上的 SVG、Markdown、csv 或 tsv 文本文件，且不是大文件。
    pub fn can_preview(&self) -> bool {
        self.preview_kind().is_some()
    }

    /// 能预览时是哪一种；不能预览时返回 `None`（见 [`Document::can_preview`]）。
    pub fn preview_kind(&self) -> Option<PreviewKind> {
        if self.unsupported.is_some() || self.title.is_some() || self.is_large() {
            return None;
        }
        let path = self.path.as_deref()?;
        if winc_preview::image::is_svg(path) {
            Some(PreviewKind::Svg)
        } else if winc_preview::markdown::is_markdown(path) {
            Some(PreviewKind::Markdown)
        } else if winc_preview::table::is_delimited(path) {
            Some(PreviewKind::Table)
        } else {
            None
        }
    }

    /// 是否正在只看预览（SVG 的图、排好版的 Markdown 或 csv 的表格）。
    pub(super) fn previewing(&self) -> bool {
        self.picture.is_some() || self.rendered || (self.table.is_some() && !self.split)
    }

    /// 文本是否可以编辑。
    ///
    /// 与 `readonly` 不同：正在预览的 SVG 在预览期间暂时只读，文本仍可编辑，切回源代码即可
    /// 修改；未保存的修改仍需备份，也可以换成磁盘版本。
    pub fn text_editable(&self) -> bool {
        if self.previewing() && self.can_preview() {
            !self.readonly_before_preview
        } else {
            !self.readonly
        }
    }

    /// 返回文档的显示方式：不能预览的文件总是 [`PreviewMode::Source`]。
    pub fn preview_mode(&self) -> PreviewMode {
        if !self.can_preview() {
            PreviewMode::Source
        } else if self.previewing() {
            PreviewMode::Preview
        } else if self.split {
            PreviewMode::Split
        } else {
            PreviewMode::Source
        }
    }

    /// 换成 `mode` 显示。
    ///
    /// 只看 SVG、表格的预览期间文档只读（看不到文字），离开时恢复原来的只读状态；Markdown 的
    /// 预览可以直接编辑，不改只读状态。不满足 [`Document::can_preview`] 时不切换，返回 `false`。
    pub fn set_preview_mode(&mut self, mode: PreviewMode) -> bool {
        let Some(kind) = self.preview_kind() else {
            return false;
        };
        if self.previewing() {
            self.picture = None;
            self.rendered = false;
            self.readonly = self.readonly_before_preview;
        }
        // 表格在只看预览和分栏时都有，换显示方式时一律重新读。
        self.table = None;
        self.split = mode == PreviewMode::Split;
        if self.split && kind == PreviewKind::Table {
            self.table = Some(TableView::loading(self.revision));
        }
        if mode == PreviewMode::Preview {
            self.readonly_before_preview = self.readonly;
            if kind != PreviewKind::Markdown {
                self.readonly = true;
            }
            match kind {
                PreviewKind::Svg => self.picture = Some(PictureState::Loading),
                PreviewKind::Markdown => self.rendered = true,
                PreviewKind::Table => self.table = Some(TableView::loading(self.revision)),
            }
        }
        true
    }

    /// 在 SVG 的源代码（或分栏）和只看预览之间切换，返回切换后是否处于预览。
    ///
    /// 不满足 [`Document::can_preview`] 时不切换，返回 `false`。
    pub fn toggle_preview(&mut self) -> bool {
        let mode = match self.preview_mode() {
            PreviewMode::Preview => PreviewMode::Source,
            PreviewMode::Source | PreviewMode::Split => PreviewMode::Preview,
        };
        self.set_preview_mode(mode) && mode == PreviewMode::Preview
    }

    /// 释放已经解码好的图，返回是否释放了；再次显示时重新解码（[`PictureState::Loading`]）。
    ///
    /// 用于不在屏幕上的标签：解码后的图片最多约 26 MB（2560 × 2560 RGBA）。
    pub fn release_picture(&mut self) -> bool {
        if !matches!(self.picture, Some(PictureState::Ready(_))) {
            return false;
        }
        self.picture = Some(PictureState::Loading);
        true
    }

    /// 接收后台解码或绘制完成的结果，返回是否采用。
    ///
    /// `revision` 是生成该结果所依据的文档版本。版本已变化（例如文件被重新读取），或当前不在
    /// [`PictureState::Loading`] 状态时丢弃结果并返回 `false`。图片文件解码失败时，文档转为
    /// 无法读取的文件（`unsupported`），用户可以「重试」。
    pub fn set_picture(&mut self, revision: u64, result: Result<Picture, String>) -> bool {
        if revision != self.revision || !matches!(self.picture, Some(PictureState::Loading)) {
            return false;
        }
        match result {
            Ok(picture) => self.picture = Some(PictureState::Ready(Arc::new(picture))),
            Err(reason) if self.unsupported.is_some() => {
                self.picture = None;
                self.unsupported = Some(Unsupported::Unreadable(reason));
            }
            Err(reason) => self.picture = Some(PictureState::Failed(reason)),
        }
        true
    }

    /// 释放读好的表格，返回是否释放了；再次显示时重新读取（[`TableState::Loading`]），工作表
    /// 和滚动位置保留。
    ///
    /// 用于不在屏幕上的标签：读好的表格最多有上百万个单元格。
    pub fn release_table(&mut self) -> bool {
        match &mut self.table {
            Some(view) if matches!(view.state, TableState::Ready(_)) => {
                view.state = TableState::Loading;
                true
            }
            _ => false,
        }
    }

    /// 需要（重新）读取表格时返回依据的文档版本；表格已经是最新的时返回 `None`。
    ///
    /// 尚未读取过，或者读好的表格落后于文档时返回当前版本，例如文本被编辑过、表格文件在磁盘上
    /// 已改变。读好的表格在按新版本读好之前继续显示。
    pub fn table_to_load(&self) -> Option<u64> {
        let view = self.table.as_ref()?;
        let stale = matches!(view.state, TableState::Loading) || view.revision != self.revision;
        stale.then_some(self.revision)
    }

    /// 接收后台读好的表格，返回是否采用。
    ///
    /// `revision` 是读取所依据的文档版本；文档已经又变了时丢弃结果并返回 `false`，随后按最新的
    /// 版本再读（见 [`Document::table_to_load`]）。
    pub fn set_table(&mut self, revision: u64, result: Result<Table, String>) -> bool {
        let Some(view) = &mut self.table else {
            return false;
        };
        if revision != self.revision {
            return false;
        }
        view.revision = revision;
        view.state = match result {
            Ok(table) => {
                view.sheet = view.sheet.min(table.sheets.len().saturating_sub(1));
                TableState::Ready(Arc::new(table))
            }
            Err(reason) => TableState::Failed(reason),
        };
        true
    }
}
