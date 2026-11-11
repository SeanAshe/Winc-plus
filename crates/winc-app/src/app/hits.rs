//! 本帧绘制的输出：各区域在绘制时记录的可点击范围与位置，供点击、悬停和滚动的命中测试使用。
//!
//! 这些数据每帧重建，不属于持久状态；没有绘制的区域在该帧没有命中范围。

use super::*;

/// 本帧绘制时记录的可点击范围与位置。
#[derive(Default)]
pub struct Hits {
    /// 开始页上可点击的区域，绘制时记录。
    ///
    /// 同一位置有两个区域时（最近列表的一行及其 ✕），✕ 排在前面。
    pub(crate) start: Vec<(Rect, StartAction)>,
    /// 无法打开的文件的占位页上，各按钮的位置。
    pub(crate) placeholder: Vec<(Rect, PlaceholderAction)>,
    /// 已折叠行末尾 `⋯` 的可点击区域和行号，绘制时记录，点击后展开。
    pub(crate) folded: Vec<(Rect, usize)>,
    /// 超长行截断说明中「显示整行」的可点击区域和行号，绘制时记录。
    pub(crate) long_lines: Vec<(Rect, usize)>,
    /// 提交视图中的可点击区域，绘制时更新。
    pub(crate) commit: Vec<(Rect, CommitTarget)>,
    /// 冲突卡片及其按钮的位置，绘制时记录；当前文档没有冲突时为 `None`。
    pub(crate) conflict_card: Option<(Rect, Vec<(Rect, ConflictAction)>)>,
    /// 侧栏标题行项目名的可点击区域，绘制时填入。
    pub(crate) project: Rect,
    /// 差异标签中的可点击区域（标题栏按钮、折叠的行），绘制时更新。
    pub(crate) diff: Vec<(Rect, DiffTarget)>,
    /// 状态栏右侧各可点击项的区域。
    pub(crate) status: Vec<(Rect, StatusItem)>,
    /// 状态栏路径中各段的可点击区域，以及段序号。
    pub(crate) crumbs: Vec<(Rect, usize)>,
    /// 文件树工具栏各按钮的位置，绘制时填入，点击时使用。
    pub(crate) tree_tools: Vec<(Rect, TreeTool)>,
    /// 工具栏收起后「⋯」按钮的位置；所有按钮都放得下时为 `None`。
    pub(crate) tree_more: Option<Rect>,
    /// 合并显示的目录在屏幕上的横向范围，绘制时测量。
    ///
    /// 各项依次为所在行的下标、左右边界（逻辑像素）和目录路径。
    pub(crate) tree_segments: Vec<(usize, f32, f32, PathBuf)>,
    /// 选择面板本帧绘制的按钮区域，点击和悬停据此判定目标。
    pub(crate) picker_parts: Vec<(Rect, PickerPart)>,
    /// 各标签的位置，按绘制顺序排列。
    pub(crate) tabs: Vec<TabHit>,
    /// 绘制时记下的可点击位置。
    pub(crate) outline: Vec<(Rect, OutlineTarget)>,
    /// 可点击的区域，绘制时更新。
    pub(crate) log: Vec<(Rect, crate::app::git::log::LogTarget)>,
    /// 可点击的区域，绘制时更新。
    pub(crate) merge: Vec<(Rect, crate::app::git::merge::MergeTarget)>,
    /// 右上角的卡片；没有时为 `None`。
    pub(crate) merge_card: Option<Rect>,
    /// 标签栏上各个按钮的位置，绘制时记下。
    pub(crate) terminal: Vec<(Rect, crate::app::terminal::HeaderHit)>,
    /// 卡片上可点击的控件，绘制时更新。
    pub(crate) changes: Vec<(Rect, ChangeTarget)>,
    /// 卡片的位置，绘制时更新；点击卡片外部时关闭卡片。
    pub(crate) changes_card: Option<Rect>,
    /// 提示条的位置，绘制时记录；提示条浮在编辑区上方，点击、悬停和右键都不穿透到下方的文字。
    pub(crate) notice_card: Option<Rect>,
    /// 提示条上按钮的位置，绘制时记录。
    pub(crate) notice_button: Option<Rect>,
    /// 上一帧绘制的链接（区域和地址），点击时据此查找。
    pub(crate) markdown_links: Vec<(winc_render::Rect, String)>,
    /// 上一帧绘制的代码块（屏幕坐标）：鼠标移入、移出时重绘，以显示或隐藏复制按钮。
    pub(crate) markdown_codes: Vec<winc_render::Rect>,
    /// 编辑区顶部钉住的各行，绘制时记录，点击和悬停时使用。
    pub(crate) sticky: Vec<StickyRow>,
    /// 上一帧绘制了复制按钮的代码块的内容；按钮在 `links` 中只记录 `COPY`，点击时才取这里的
    /// 文字。
    ///
    /// 与排版结果共用同一段文字：鼠标停在代码块上时每帧都要绘制按钮，无需每帧复制整块。
    pub(crate) markdown_copy: Option<std::sync::Arc<str>>,
    /// 上一帧绘制了全屏按钮的图的源码。
    ///
    /// `links` 中的按钮只记录 `EXPAND`，点击时从这里取源码，绘制时不必复制源码。
    pub(crate) markdown_expand: Option<std::sync::Arc<str>>,
    /// 上一帧绘制了滚动条的可横向滚动的表格；没有时为 `None`。
    ///
    /// 同一时刻只显示一个表格的滚动条：鼠标所在的表格，或最近一次横向滚动的表格。
    pub(crate) markdown_table_bar: Option<crate::app::preview::markdown::TableBar>,
    /// 上一帧绘制的标题指示条和标题列表的可点击位置，见 `view::markdown` 的 `draw_toc`。
    pub(crate) markdown_toc: Vec<(winc_render::Rect, crate::app::preview::markdown::TocTarget)>,
    /// 上一帧标题指示条列出的各标题在内容里的高度（逻辑像素），与 [`TocTarget::Heading`] 的
    /// 序号对应；点击标题时滚动到该处。
    pub(crate) markdown_toc_tops: Vec<f32>,
    /// PDF 预览缩略图栏中各页的位置，以及点击后正文滚动到的位置（逻辑像素）。
    pub(crate) pdf_thumbs: Vec<(Rect, f32)>,
    /// PDF 预览右下角工具栏上各按钮的位置。
    pub(crate) pdf_tools: Vec<(Rect, crate::app::preview::pdf::PdfTool)>,
    /// 当前帧绘制出的「安装方法…」链接的位置，以及链接所属的设置项。
    ///
    /// 文字宽度要到绘制时才知道，悬停和点击使用这里记录的位置。
    pub(crate) install_links: Vec<(crate::layout::Setting, Rect)>,
}

impl Hits {
    /// 清空全部记录，保留各列表已分配的内存；每帧开始绘制前调用。
    ///
    /// 字段在此逐个列出：新增字段而未在此处理时无法通过编译。
    pub(crate) fn clear(&mut self) {
        let Hits {
            start,
            placeholder,
            folded,
            long_lines,
            commit,
            conflict_card,
            project,
            diff,
            status,
            crumbs,
            tree_tools,
            tree_more,
            tree_segments,
            picker_parts,
            tabs,
            outline,
            log,
            merge,
            merge_card,
            terminal,
            changes,
            changes_card,
            notice_card,
            notice_button,
            markdown_links,
            markdown_codes,
            sticky,
            markdown_copy,
            markdown_expand,
            markdown_table_bar,
            markdown_toc,
            markdown_toc_tops,
            pdf_thumbs,
            pdf_tools,
            install_links,
        } = self;
        start.clear();
        placeholder.clear();
        folded.clear();
        long_lines.clear();
        commit.clear();
        *conflict_card = None;
        *project = Rect::default();
        diff.clear();
        status.clear();
        crumbs.clear();
        tree_tools.clear();
        *tree_more = None;
        tree_segments.clear();
        picker_parts.clear();
        tabs.clear();
        outline.clear();
        log.clear();
        merge.clear();
        *merge_card = None;
        terminal.clear();
        changes.clear();
        *changes_card = None;
        *notice_card = None;
        *notice_button = None;
        markdown_links.clear();
        markdown_codes.clear();
        sticky.clear();
        *markdown_copy = None;
        *markdown_expand = None;
        *markdown_table_bar = None;
        markdown_toc.clear();
        markdown_toc_tops.clear();
        pdf_thumbs.clear();
        pdf_tools.clear();
        install_links.clear();
    }
}
