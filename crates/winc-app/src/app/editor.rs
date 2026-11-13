//! 编辑区的行为：编辑命令、多光标、折叠、粘滞滚动、查找、转到定义、符号、后退前进、书签、
//! 粘贴、格式化、滚动条、提示气泡和磁盘冲突。
//!
//! 每个子模块负责一类命令，由 [`App`] 上对应的方法调用；文档状态保存在 [`App`] 的字段里。

use super::*;

pub(super) mod bookmarks;
pub(crate) mod clipboard;
pub(super) mod conflict;
pub(super) mod cursors;
pub(crate) mod definition;
pub(super) mod editing;
pub(super) mod find;
pub(super) mod folding;
pub(super) mod format;
pub(super) mod hint;
pub(super) mod navigation;
pub(super) mod paste;
pub(super) mod scrollbar;
pub(super) mod sticky;
pub(super) mod symbols;

/// 编辑区跨帧复用的缓存和待采用的计算结果。
pub(crate) struct EditorCaches {
    /// 粘滞滚动的计算缓存：钉住的行未变化时不重新计算。
    pub(in crate::app) sticky_cache: Option<editor::sticky::StickyCache>,
    /// 已完成、待采用的大文件断行结果（见 [`App::poll_wraps`]）。
    pub(in crate::app) wraps: Arc<Mutex<Vec<(DocumentId, winc_editor::WrapResult)>>>,
    /// 长行的着色结果，跨帧复用（见 [`crate::view::LineTokens`]）。
    pub(crate) line_tokens: std::cell::RefCell<crate::view::LineTokens>,
    /// 缩略图中各行的小块，跨帧复用（见 [`crate::view::MinimapRows`]）。
    pub(crate) minimap_rows: std::cell::RefCell<crate::view::MinimapRows>,
    /// 最近一次计算的最大横向滚动量及其依据的条件（见 [`App::text_max_scroll_x`]）。
    pub(in crate::app) max_scroll_x: Option<(editor::editing::MaxScrollKey, f32)>,
    /// 各行的显示宽度，供 [`App::text_max_scroll_x`] 复用。
    pub(in crate::app) line_widths: editor::editing::LineWidths,
    /// 滚动条上标出的光标所在词的缓存。
    pub(crate) ruler_cache: Option<crate::view::RulerCache>,
    /// 高亮状态是否尚未推算到可见区域；为真时需要再绘制一帧。
    pub highlight_pending: bool,
    /// 光标旁括号的配对结果缓存。
    pub brace_cache: Option<(BraceKey, Option<(usize, usize)>)>,
}

/// 编辑区顶部的输入条：查找条与跳转条。
pub(crate) struct InputBars {
    /// 当前查找的内容、各文档里已找到的匹配和替换状态。
    pub search: Search,
    /// 查找条的界面状态。
    pub find: editor::find::FindBar,
    /// 跳转条（⌃G）的输入；`None` 表示跳转条未打开。
    pub goto_input: Option<TextField>,
    /// 跳转条的当前用途。
    pub prompt_kind: PromptKind,
}

/// 跳转：位置历史、后台的定义查找和名字旁的提示。
pub(crate) struct Jumps {
    /// 光标所在名字旁边的提示气泡（例如没有找到定义）；没有时为 `None`。
    pub(crate) name_hint: Option<NameHint>,
    /// 「后退」「前进」使用的位置历史。
    pub(in crate::app) navigation: editor::navigation::Navigation,
    /// 正在后台进行的项目内定义查找。
    pub(in crate::app) definitions: Option<editor::definition::DefinitionSearch>,
}
