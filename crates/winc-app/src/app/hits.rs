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
    /// Git 面板标题行中可点击的区域（日志的标签和按钮），绘制时更新。
    pub(crate) git_panel: Vec<(Rect, crate::app::git::panel::PanelHit)>,
    /// 可点击的区域，绘制时更新。
    pub(crate) merge: Vec<(Rect, crate::app::git::merge::MergeTarget)>,
    /// 右上角的卡片；没有时为 `None`。
    pub(crate) merge_card: Option<Rect>,
    /// 标签栏上各个按钮的位置，绘制时记下。
    pub(crate) terminal: Vec<(Rect, crate::app::terminal::HeaderHit)>,
    /// 分栏时各栏标签栏上的按钮，按栏的排列顺序；正在用的一栏的同时在 `terminal` 里。
    pub(crate) terminal_groups: Vec<Vec<(Rect, crate::app::terminal::HeaderHit)>>,
    /// 「未提交的更改」标签上可以点的地方，绘制时记下。
    pub(crate) combined: Vec<(Rect, crate::app::git::combined::CombinedHit)>,
    /// 卡片上可点击的控件，绘制时更新。
    pub(crate) changes: Vec<(Rect, ChangeTarget)>,
    /// 卡片的位置，绘制时更新；点击卡片外部时关闭卡片。
    pub(crate) changes_card: Option<Rect>,
    /// 提示条的位置，绘制时记录；提示条浮在编辑区上方，点击、悬停和右键都不穿透到下方的文字。
    pub(crate) notice_card: Option<Rect>,
    /// 提示条上按钮的位置，绘制时记录。
    pub(crate) notice_button: Option<Rect>,
    /// 跳转条或 Vim 命令行的位置，绘制时记录。跳转条按前台的布局只绘制一次，其他窗格不能按
    /// 自身的编辑区推算它的位置。
    pub(crate) prompt_bar: Option<Rect>,
    /// 上一帧绘制的链接（区域和地址），点击时据此查找。
    pub(crate) markdown_links: Vec<(winc_render::Rect, String)>,
    /// 上一帧在可编辑的预览中绘制的、原文可以改写的图片。
    pub(crate) markdown_images: Vec<crate::app::preview::markdown::resize::ImageAt>,
    /// 上一帧绘制的本地图片及其文件。
    pub(crate) markdown_pictures: Vec<(winc_render::Rect, std::path::PathBuf)>,
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
    /// 上一帧绘制的格式工具栏；没有显示时为 `None`。
    pub(crate) selection_card: Option<Rect>,
    /// 上一帧绘制的格式工具栏中各按钮的位置。
    pub(crate) selection_tools: Vec<(Rect, crate::app::preview::markdown::toolbar::Tool)>,
    /// 上一帧绘制的链接地址补全列表的卡片；没有画出列表时为 `None`。
    pub(crate) suggest_card: Option<Rect>,
    /// 上一帧绘制的链接地址补全列表的各行：位置和行号。
    pub(crate) suggest: Vec<(Rect, usize)>,
    /// 上一帧绘制的标题指示条和标题列表的可点击位置，见 `view::markdown` 的 `draw_toc`。
    pub(crate) markdown_toc: Vec<(winc_render::Rect, crate::app::preview::markdown::TocTarget)>,
    /// 上一帧标题指示条列出的各标题在内容里的高度（逻辑像素），与 [`TocTarget::Heading`] 的
    /// 序号对应；点击标题时滚动到该处。
    pub(crate) markdown_toc_tops: Vec<f32>,
    /// 标题指示条列出的各标题所在的顶层块，与 `markdown_toc_tops` 对应。
    pub(crate) markdown_toc_blocks: Vec<usize>,
    /// PDF 预览缩略图栏中各页的位置，以及点击后正文滚动到的位置（逻辑像素）。
    pub(crate) pdf_thumbs: Vec<(Rect, f32)>,
    /// PDF 预览右下角工具栏上各按钮的位置。
    pub(crate) pdf_tools: Vec<(Rect, crate::app::preview::pdf::PdfTool)>,
    /// LaTeX 预览右上角编译状态的位置；没有错误时为空矩形。
    pub(crate) latex_status: Rect,
    /// 当前帧绘制出的「安装方法…」链接的位置，以及链接所属的设置项。
    ///
    /// 文字宽度要到绘制时才知道，悬停和点击使用这里记录的位置。
    pub(crate) install_links: Vec<(crate::layout::Setting, Rect)>,
    /// 当前帧绘制出的「恢复默认」的位置，以及它所属的设置项；位置随名称的宽度而定。
    pub(crate) reset_links: Vec<(crate::layout::Setting, Rect)>,
    /// 当前帧「快捷键」页中绘制出的按钮、链接和键帽上的 ×；位置随文字宽度而定，悬停和点击用
    /// 这里的。
    pub(crate) keys_parts: Vec<(Rect, crate::layout::SettingsPart)>,
    /// 标签页切换列表中各行的位置和行号（见 `SwitchList::rows`）。
    pub(crate) tab_switch: Vec<(Rect, usize)>,
    /// `.http` 文件行号栏中发送请求的按钮的位置和请求行的行号（从 0 起）。
    pub(crate) http_runs: Vec<(Rect, usize)>,
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
            git_panel,
            merge,
            merge_card,
            terminal,
            terminal_groups,
            combined,
            changes,
            changes_card,
            notice_card,
            notice_button,
            prompt_bar,
            markdown_links,
            markdown_images,
            markdown_pictures,
            markdown_codes,
            sticky,
            markdown_copy,
            markdown_expand,
            markdown_table_bar,
            markdown_toc,
            markdown_toc_tops,
            markdown_toc_blocks,
            suggest_card,
            suggest,
            selection_card,
            selection_tools,
            pdf_thumbs,
            pdf_tools,
            latex_status,
            install_links,
            reset_links,
            keys_parts,
            tab_switch,
            http_runs,
        } = self;
        start.clear();
        *suggest_card = None;
        *selection_card = None;
        selection_tools.clear();
        suggest.clear();
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
        git_panel.clear();
        merge.clear();
        *merge_card = None;
        terminal.clear();
        terminal_groups.clear();
        combined.clear();
        changes.clear();
        *changes_card = None;
        *notice_card = None;
        *notice_button = None;
        *prompt_bar = None;
        markdown_links.clear();
        markdown_images.clear();
        markdown_pictures.clear();
        markdown_codes.clear();
        sticky.clear();
        *markdown_copy = None;
        *markdown_expand = None;
        *markdown_table_bar = None;
        markdown_toc.clear();
        markdown_toc_tops.clear();
        markdown_toc_blocks.clear();
        pdf_thumbs.clear();
        pdf_tools.clear();
        *latex_status = Rect::default();
        install_links.clear();
        reset_links.clear();
        keys_parts.clear();
        tab_switch.clear();
        http_runs.clear();
    }
}
