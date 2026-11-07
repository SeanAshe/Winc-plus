//! `layout` 的测试：命中的区域取靠前一条，快速打开面板与查找条的绘制位置点击相容，文件树与侧栏
//! 在动画中的位置，拖动分隔条不回跳，滑块与滚动的换算，设置页的搜索框与整页布局一致。

use super::*;

#[test]
fn hit_takes_the_first_region_under_the_point() {
    let regions = [
        (Rect::new(0.0, 0.0, 10.0, 10.0), 'a'),
        (Rect::new(5.0, 5.0, 10.0, 10.0), 'b'),
    ];
    assert_eq!(hit(&regions, 7.0, 7.0), Some('a'));
    assert_eq!(hit(&regions, 12.0, 12.0), Some('b'));
    assert_eq!(hit(&regions, 30.0, 30.0), None);
}

#[test]
fn picker_rows_are_drawn_where_they_are_clicked() {
    let card = Rect::new(
        100.0,
        50.0,
        560.0,
        BAR_HEIGHT + 5.0 * PICKER_ROW + 2.0 * PICKER_PAD,
    );
    let parts = PickerParts::new(card);
    // 第一条与输入框之间、最后一条与卡片底边之间都留有 `PICKER_PAD` 的空白。
    assert_eq!(parts.row(0, 0.0).y, card.y + BAR_HEIGHT + PICKER_PAD);
    assert_eq!(parts.row(4, 0.0).bottom(), card.bottom() - PICKER_PAD);

    // 滚动 2.5 行：第 2 条（从 0 起）只露出下半截，贴着列表顶边；第 7 条只露出上半截。
    let scroll = 2.5 * PICKER_ROW;
    assert_eq!(parts.visible(scroll, 100), 2..8);
    assert_eq!(parts.row(2, scroll).y, parts.list.y - PICKER_ROW / 2.0);
    assert_eq!(parts.row(3, scroll).y, parts.list.y + PICKER_ROW / 2.0);

    // 每一条结果的绘制位置，点击其中点时命中的就是该条。
    for index in parts.visible(scroll, 100) {
        let row = parts.row(index, scroll).intersect(parts.list);
        assert_eq!(parts.row_at(row.y + row.h / 2.0, scroll), Some(index));
    }
    assert_eq!(parts.row_at(parts.list.y - 1.0, scroll), None);
    assert_eq!(parts.row_at(parts.list.bottom(), scroll), None);

    // 结果比列表短时只到最后一条。
    assert_eq!(parts.visible(0.0, 3), 0..3);
    assert_eq!(parts.visible(0.0, 0), 0..0);
}

#[test]
fn scroll_to_show_moves_only_as_far_as_needed() {
    // 可见范围 100..200。
    assert_eq!(scroll_to_show(100.0, 120.0, 140.0, 100.0), 100.0);
    assert_eq!(scroll_to_show(100.0, 80.0, 100.0, 100.0), 80.0);
    assert_eq!(scroll_to_show(100.0, 190.0, 210.0, 100.0), 110.0);
}

#[test]
fn thumb_spans_track_from_top_to_bottom() {
    let track = Rect::new(0.0, 100.0, 10.0, 200.0);
    assert!(scrollbar_thumb(track, 0.0, 150.0).is_none());

    // 滑块的活动范围比轨道两端各短 `SCROLLBAR_END`，长度按「可见 / 内容」的比例分配。
    let room = 200.0 - 2.0 * SCROLLBAR_END;
    let top = scrollbar_thumb(track, 0.0, 800.0).unwrap();
    let bottom = scrollbar_thumb(track, 600.0, 800.0).unwrap();
    assert_eq!(top.y, 100.0 + SCROLLBAR_END);
    assert_eq!(top.h, room / 4.0);
    assert_eq!(bottom.bottom(), 300.0 - SCROLLBAR_END);

    // 水平的滑块贴着下边。内容放得下时没有滑块。
    let wide = Rect::new(0.0, 100.0, 200.0, 400.0);
    assert!(hscrollbar_thumb(wide, 0.0, 150.0).is_none());

    // 滑块的长度按「可见宽度 / 内容宽度」的比例计算。
    let left = hscrollbar_thumb(wide, 0.0, 400.0).unwrap();
    assert_eq!((left.x, left.w), (SCROLLBAR_END, room / 2.0));
    assert_eq!(hscrollbar_thumb(wide, 0.0, 200.0), None);
    // 紧贴着下边，只留 `SCROLLBAR_INSET`。
    assert_eq!(left.bottom(), 500.0 - SCROLLBAR_INSET);

    // 滚动量最大为「内容宽度减文件树的宽度」，滚到底时滑块右端停在活动范围的右端。
    let end = hscrollbar_thumb(wide, 200.0, 400.0).unwrap();
    assert_eq!(end.right(), 200.0 - SCROLLBAR_END);

    // 拖动按同样的比例：点在滑块左端，拖到滑块贴住右端时滚到底。
    assert_eq!(
        scroll_for_thumb(0.0, 200.0, end.w, 100.0, 0.0, 200.0),
        200.0
    );
    assert_eq!(
        scroll_for_thumb(100.0, 200.0, 50.0, 400.0, 0.0, 600.0),
        600.0
    );
    assert_eq!(scroll_for_thumb(100.0, 200.0, 50.0, 100.0, 0.0, 600.0), 0.0);
}

#[test]
fn tree_tools_end_at_header_right() {
    let header = Rect::new(0.0, 40.0, 260.0, 32.0);
    let (tools, more) = tree_tools(header, 90.0);
    assert!(more.is_none());
    assert_eq!(tools.len(), 5);
    assert_eq!(tools[4].0.right(), 252.0);
    assert_eq!(tools[0].1, TreeTool::NewFile);
    assert!(
        tools
            .windows(2)
            .all(|w| w[0].0.right() + BUTTON_GAP == w[1].0.x)
    );
}

#[test]
fn tree_tools_fold_from_the_right_into_more() {
    let header = Rect::new(0.0, 40.0, 260.0, 32.0);
    let right = header.right() - 8.0;

    // 每少一个格子收起一个，从右往左收：定位最先进「⋯」，新建文件最后。
    for (room, kept) in [(4, 3), (3, 2), (2, 1), (1, 0)] {
        let name_right = right - room as f32 * (BUTTON_SMALL + BUTTON_GAP) + BUTTON_GAP;
        let (tools, more) = tree_tools(header, name_right);
        let more = more.unwrap();
        assert_eq!(tools.len(), kept);
        assert_eq!(more.right(), right);
        assert!(
            tools
                .iter()
                .all(|(r, _)| r.x >= name_right && r.right() <= more.x)
        );
        let order: Vec<_> = tools.iter().map(|&(_, t)| t).collect();
        assert_eq!(order, TreeTool::ALL[..kept]);
        assert_eq!(folded_tools(kept), &TreeTool::ALL[kept..]);
    }

    // 一个格子都没有：工具栏整个不画，项目名优先。
    let (tools, more) = tree_tools(header, right - 10.0);
    assert!(tools.is_empty() && more.is_none());
}

#[test]
fn folding_tools_slide_into_more() {
    let header = Rect::new(0.0, 40.0, 260.0, 32.0);
    let right = header.right() - 8.0;
    let step = BUTTON_SMALL + BUTTON_GAP;

    // 停下来时与终点布局一致：全部显示，或者留三个加「⋯」。
    let (all, _) = tree_tools(header, 0.0);
    let (tools, more) = folding_tools(header, &[1.0; 5], 0.0);
    assert!(more.is_none());
    assert_eq!(
        tools.iter().map(|&(r, t, _)| (r, t)).collect::<Vec<_>>(),
        all
    );
    let (kept, end_more) = tree_tools(header, right - 4.0 * step + BUTTON_GAP);
    let (tools, more) = folding_tools(header, &[1.0, 1.0, 1.0, 0.0, 0.0], 1.0);
    assert_eq!(more, end_more);
    assert_eq!(
        tools.iter().map(|&(r, t, _)| (r, t)).collect::<Vec<_>>(),
        kept
    );

    // 收到一半：定位本来就在「⋯」那一格，和「⋯」叠在一起交替淡入淡出；全部折叠往右挪向
    // 「⋯」；左边的按钮跟着右移，但还没到终点。
    let (tools, more) = folding_tools(header, &[1.0, 1.0, 1.0, 0.5, 0.5], 0.5);
    let more = more.unwrap();
    assert_eq!(tools.len(), 5);
    assert_eq!(tools[4].0.x, more.x);
    let collapse = tools[3].0;
    assert!(collapse.x > all[3].0.x && collapse.x < more.x);
    assert!(tools[0].0.x > all[0].0.x && tools[0].0.x < kept[0].0.x);

    // 快收完时，全部折叠也几乎叠在「⋯」上。
    let (tools, _) = folding_tools(header, &[1.0, 1.0, 1.0, 0.01, 0.01], 1.0);
    assert!((tools[3].0.x - more.x).abs() < 1.0);
}

#[test]
fn the_sidebar_toggle_stays_next_to_the_traffic_lights() {
    // 侧栏展开、收起、动画途中，开关都在同一个位置：图标可见的左边缘与红绿灯右端相隔
    // `LIGHTS_GAP`；macOS 26 的红绿灯右端是 80。
    let layouts = [1.0, 0.5, 0.1, 0.0]
        .map(|shown| Layout::compute(1000.0, 600.0, 260.0, shown, 0, 0, 40.0, STATUS_HEIGHT));
    for l in &layouts {
        assert_eq!(l.sidebar_button, layouts[0].sidebar_button);
    }
    let toggle = layouts[0].sidebar_button.x + (BUTTON - icons::SIDEBAR_WIDTH) / 2.0;
    if cfg!(target_os = "macos") {
        assert_eq!(traffic_lights_end(), 80.0);
        assert_eq!(toggle, 80.0 + LIGHTS_GAP);
    }

    // 视图切换靠侧栏右端（底槽离右边 8），相邻两段隔一道缝，和开关垂直居中；收起时标签从
    // 开关右边开始。
    let (open, closed) = (layouts[0], layouts[3]);
    let tabs = view_tabs(
        open.sidebar_button,
        open.sidebar,
        &BASE_VIEWS.map(|v| (v, 1.0)),
    );
    let (files, search) = (tabs[0].0, tabs[1].0);
    let last = tabs.last().unwrap().0;
    assert_eq!(last.right() + 2.0, open.sidebar.right() - 8.0);
    assert_eq!(search.x, files.right() + SEGMENT_SPACING);
    assert_eq!(
        files.y + files.h / 2.0,
        open.sidebar_button.y + open.sidebar_button.h / 2.0
    );
    // 侧栏最窄时也放得下：不压到开关上，不伸出侧栏。
    let narrow = Layout::compute(
        1000.0,
        600.0,
        MIN_PANEL_WIDTH,
        1.0,
        0,
        0,
        40.0,
        STATUS_HEIGHT,
    );
    let all = [SidebarView::Files, SidebarView::Search, SidebarView::Commit].map(|v| (v, 1.0));
    let tabs = view_tabs(narrow.sidebar_button, narrow.sidebar, &all);
    assert!(tabs[0].0.x - 2.0 >= narrow.sidebar_button.right() + SEGMENT_GAP);
    assert!(tabs.last().unwrap().0.right() + 2.0 <= narrow.sidebar.right() - 8.0);
    // 「提交」一段出现到一半：它和前面的缝都只有一半宽，整个控件仍靠右，前两段往右让出一半。
    let mut growing = all;
    growing[2].1 = 0.5;
    let half = view_tabs(open.sidebar_button, open.sidebar, &growing);
    let full = view_tabs(open.sidebar_button, open.sidebar, &all);
    assert_eq!(half[2].0.w, SEGMENT_W / 2.0);
    assert_eq!(half[2].0.right(), full[2].0.right());
    assert_eq!(half[2].0.x, half[1].0.right() + SEGMENT_SPACING / 2.0);
    assert_eq!(
        half[0].0.x - full[0].0.x,
        (SEGMENT_W + SEGMENT_SPACING) / 2.0
    );
    assert_eq!(closed.tabs.x - closed.sidebar_button.right(), TAB_GAP);
}

#[test]
fn hidden_sidebar_moves_button_next_to_traffic_lights() {
    let layout = Layout::compute(1000.0, 600.0, 260.0, 0.0, 0, 0, 40.0, STATUS_HEIGHT);
    assert!(layout.sidebar.right() <= 0.0);
    assert_eq!(layout.editor.x, ISLAND_MARGIN);
    assert_eq!(
        layout.sidebar_button.x,
        first_button_x(icons::SIDEBAR_WIDTH)
    );
    assert!(layout.tabs.x > layout.sidebar_button.right());
    // 顶栏右端不放按钮，标签栏延伸到编辑区卡片的右端，只留出边距。
    assert_eq!(layout.tabs.right(), layout.top_bar.right() - 8.0);
    assert_eq!(
        layout.top_bar.right(),
        layout.window.right() - ISLAND_MARGIN
    );
    assert_eq!(layout.editor.bottom(), layout.status.y);
}

#[test]
fn sidebar_slides_during_animation() {
    let open = Layout::compute(1000.0, 600.0, 260.0, 1.0, 0, 0, 40.0, STATUS_HEIGHT);
    let half = Layout::compute(1000.0, 600.0, 260.0, 0.5, 0, 0, 40.0, STATUS_HEIGHT);
    let tiny = Layout::compute(1000.0, 600.0, 260.0, 0.1, 0, 0, 40.0, STATUS_HEIGHT);
    let closed = Layout::compute(1000.0, 600.0, 260.0, 0.0, 0, 0, 40.0, STATUS_HEIGHT);

    // 侧栏卡片保持原宽滑出，与编辑区卡片之间始终隔 `ISLAND_GAP`。
    let card_w = 260.0 - ISLAND_MARGIN - ISLAND_GAP / 2.0;
    assert_eq!(
        (half.sidebar.w, half.sidebar.right()),
        (card_w, 130.0 - ISLAND_GAP / 2.0)
    );
    assert_eq!(half.editor.x - half.sidebar.right(), ISLAND_GAP);
    assert_eq!(half.splitter.w, 0.0);
    assert!(open.splitter.w > 0.0);

    // 开关不动；标签始终在开关右侧，位置不跳变。
    assert_eq!(tiny.sidebar_button.x, closed.sidebar_button.x);
    for l in [open, half, tiny, closed] {
        assert!(l.tabs.x >= l.sidebar_button.right() + TAB_GAP);
        assert!(l.tabs.x >= l.editor.x + TAB_INSET);
    }
    assert!(open.tabs.x > half.tabs.x && half.tabs.x > closed.tabs.x);
}

#[test]
fn dragging_the_splitter_keeps_the_sidebar_in_place() {
    // 拖动分隔条时宽度带小数（来自鼠标位置）。侧栏卡片的左边缘始终离窗口左边缘
    // `ISLAND_MARGIN`，右边缘与编辑区卡片相隔 `ISLAND_GAP`，都在整像素上；宽度逐步增大时编辑区
    // 只向右移动，不会回跳。
    let mut last = 0.0;
    for i in 0..=40 {
        let width = 260.0 + i as f32 * 0.125;
        let l = Layout::compute(1000.0, 600.0, width, 1.0, 0, 0, 40.0, STATUS_HEIGHT);
        for rect in [l.sidebar, l.sidebar_header, l.tree] {
            assert_eq!(rect.x, ISLAND_MARGIN, "{width}");
        }
        assert_eq!(l.sidebar.right() + ISLAND_GAP, l.editor.x, "{width}");
        assert_eq!(l.editor.x, l.editor.x.round(), "{width}");
        assert!(l.editor.x >= last, "{width}");
        last = l.editor.x;
    }
}

#[test]
fn tree_rows_during_folder_animation() {
    let tree = Rect::new(0.0, 100.0, 200.0, 400.0);
    // 第 2 行（从 0 起）的目录有 3 个子行，即第 3 到 5 行。
    let block = Some((3, 6));
    let slot = |v, hidden| tree_row_slot(tree, 0.0, v, block, hidden);

    // 刚开始展开：子行全部隐藏，其后的行仍在展开前的位置。
    let (row, clip) = slot(4, 3.0 * TREE_ROW);
    assert_eq!(row.y, 100.0 + 4.0 * TREE_ROW);
    assert_eq!(clip.h, 0.0);
    assert_eq!(slot(6, 3.0 * TREE_ROW).0.y, 100.0 + 3.0 * TREE_ROW);
    assert_eq!(slot(2, 3.0 * TREE_ROW).0.y, 100.0 + 2.0 * TREE_ROW);

    // 露出一半：已露出一行半，其后的行相应下移一行半。
    let (_, clip) = slot(3, 1.5 * TREE_ROW);
    assert_eq!((clip.y, clip.h), (100.0 + 3.0 * TREE_ROW, 1.5 * TREE_ROW));
    assert_eq!(slot(6, 1.5 * TREE_ROW).0.y, 100.0 + 4.5 * TREE_ROW);

    // 动画结束（`hidden` 为 0）：与没有动画时相同。
    for v in 0..10 {
        assert_eq!(slot(v, 0.0).0, tree_row_slot(tree, 0.0, v, None, 0.0).0);
    }
}

#[test]
fn search_floats_inside_editor() {
    let layout = Layout::compute(1000.0, 600.0, 260.0, 1.0, 1, 0, 40.0, STATUS_HEIGHT);
    assert!(layout.search.x > layout.editor.x && layout.search.right() < layout.editor.right());
    assert_eq!(layout.text.y, layout.editor.y);
    assert!(layout.sidebar_button.right() <= layout.sidebar.right());

    // 部件从左到右排开，互不重叠，都在查找条里。
    let bar = layout.search;
    let parts = SearchParts::new(bar);
    let row = [
        parts.expand,
        parts.field,
        parts.case_toggle,
        parts.word_toggle,
        parts.regex_toggle,
        parts.divider,
        parts.prev,
        parts.next,
        parts.filter,
        parts.close,
    ];
    assert!(row[0].x > bar.x && row[9].right() < bar.right());
    assert!(row.windows(2).all(|w| w[0].right() <= w[1].x));
    assert_eq!(parts.replace_field.w, 0.0);

    let (cx, cy) = (parts.next.x + 12.0, parts.next.y + 12.0);
    assert_eq!(parts.button_at(cx, cy), Some(SearchButton::Next));
    assert_eq!(parts.button_at(parts.field.x + 20.0, cy), None);

    // 展开替换后第二行在第一行下面，按钮和第一行右端对齐。
    let layout = Layout::compute(1000.0, 600.0, 260.0, 1.0, 2, 0, 40.0, STATUS_HEIGHT);
    let parts = SearchParts::new(layout.search);
    assert_eq!(layout.search.h, 2.0 * BAR_HEIGHT);
    assert_eq!(parts.replace_field.y, layout.search.y + BAR_HEIGHT);
    assert!(parts.replace_field.right() <= parts.replace_one.x);
    assert!(parts.replace_one.right() <= parts.replace_all.x);
    assert_eq!(parts.replace_all.right(), parts.close.right());
    assert_eq!(parts.replace_one.x, parts.filter.x);
    assert_eq!(parts.replace_all.x, parts.close.x);
    let (cx, cy) = (parts.replace_all.x + 12.0, parts.replace_all.y + 12.0);
    assert_eq!(parts.button_at(cx, cy), Some(SearchButton::ReplaceAll));
}

#[test]
fn keys_header_matches_the_full_keys_layout() {
    // 悬停只算搜索框和「录制按键」，位置必须与整页布局一致，与列表内容和滚动无关。
    let body = Rect::new(200.0, 60.0, 640.0, 500.0);
    let groups = keymap::current_rows()
        .iter()
        .map(|(group, rows)| (*group, rows.iter().collect()))
        .collect();
    let keys = KeysLayout::compute(body, groups, 120.0);
    assert!(!keys.rows.is_empty());
    assert_eq!(KeysLayout::header(body), (keys.search, keys.record));
    assert!(keys.search.contains(keys.record.x, keys.record.y));
}

#[test]
fn the_minimap_moves_only_when_the_document_does_not_fit() {
    assert_eq!(minimap_first_row(100, 200, 50.0, 80.0), 0);
    assert_eq!(minimap_first_row(1000, 200, 0.0, 950.0), 0);
    assert_eq!(minimap_first_row(1000, 200, 950.0, 950.0), 800);
    assert_eq!(minimap_first_row(1000, 200, 475.0, 950.0), 400);
}
