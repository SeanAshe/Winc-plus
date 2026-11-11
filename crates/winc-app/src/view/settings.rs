//! 设置浮层（⌘,）的绘制：盖住整个窗口的模态卡片，左侧是分类栏，右侧是当前页。
//!
//! 分类、滚动位置和控件状态来自 `app::settings`；本模块按 [`SettingsLayout`] 给出的
//! 几何绘制，并记录当前页中各控件的可点击区域。

mod keys;

use std::borrow::Cow;

use winc_render::{Color, FontKind, Frame, Rect};

use super::*;
use crate::app::DialogCache;
use crate::app::Focus;
use crate::icons::Direction;
use crate::layout::{
    ProductParts, Setting, SettingControl, SettingRow, SettingsLayout, SettingsPage, SettingsPart,
    StepperParts, THEME_CHOICES, settings_thumb, theme_segment, theme_thumb, theme_tile,
};
use crate::theme::themes;
use winc_config::ThemeChoice;
use winc_i18n::t;

/// 返回设置浮层的布局：绘制期间渲染器不在 `App` 中，说明文字改用 `frame` 量宽度。
fn layout(app: &App, frame: &mut Frame<'_>) -> SettingsLayout {
    let size = app.fonts.ui_small;
    app.settings_layout_with(&mut |text: &str| frame.measure(FontKind::Ui, size, text))
}

/// 绘制设置浮层：遮罩、卡片、分类栏、标题和当前页的内容。
pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    let layout = layout(app, frame);
    if app.settings.motion.is_some() {
        // 动画开头：底下的窗口已绘制完成，保存为快照；再绘制一张原大、不透明的卡片并保存，
        // 之后恢复底下的窗口。后续每帧直接用这两张快照（`draw_cached`），不必每帧重绘整个窗口
        // 和整张卡片（在 2× 缩放的屏幕上每帧需要二三十毫秒，动画只有几帧，会明显掉帧）。
        let base = frame.snapshot(frame.bounds());
        draw_dialog(app, frame, &layout);
        let card = frame.snapshot(layout.card);
        app.settings.cache = Some(DialogCache { base, card });
        draw_motion(app, frame, &layout);
        return;
    }

    // 遮罩颜色很淡，只略微压暗下层内容，卡片自身靠阴影浮起；卡片覆盖的区域不需要再铺遮罩。
    let p = app.palette;
    let covered = frame.covered(layout.card, RADIUS_CARD);
    frame.rect_except(app.layout.window, p.scrim, 0.0, covered);
    // 开始滚动时保存已绘制的窗口和遮罩，滚动期间不再重绘（`draw_scrolling`）。
    if app.settings.scrolled_at.is_some() && app.settings.scroll_base.is_none() {
        app.settings.scroll_base = Some(frame.snapshot(frame.bounds()));
    }
    super::draw_card_frame(frame, p, layout.card, RADIUS_CARD);
    draw_dialog(app, frame, &layout);
}

/// 绘制设置页滚动期间的一帧：恢复保存的窗口画面（含遮罩），只重绘卡片，返回 `true`。
///
/// 没有快照，或窗口尺寸、缩放比例已变化时返回 `false`，由调用方完整绘制并重新保存快照。
pub(crate) fn draw_scrolling(app: &mut App, frame: &mut Frame<'_>) -> bool {
    if !app.settings_reuses_base(frame.bounds(), frame.scale()) {
        return false;
    }
    let Some(base) = app.settings.scroll_base.take() else {
        return false;
    };
    // `tint` 全透明时 `restore_tinted` 原样恢复快照。
    frame.restore_tinted(&base, app.palette.scrim.alpha(0.0));
    app.settings.scroll_base = Some(base);
    // 与完整绘制时相同，卡片不透出背景图。
    frame.end_wallpaper();
    let layout = layout(app, frame);
    super::draw_card_frame(frame, app.palette, layout.card, RADIUS_CARD);
    draw_dialog(app, frame, &layout);
    true
}

/// 设置浮层出现、消失的动画途中：有开头存下的快照时，直接拿来绘制这一帧，返回 `true`；没有
/// 快照或窗口尺寸、缩放倍数变化时返回 `false`，照常绘制（绘制时重新保存快照）。
pub(crate) fn draw_cached(app: &mut App, frame: &mut Frame<'_>) -> bool {
    if app.settings.motion.is_none() {
        return false;
    }
    let layout = layout(app, frame);
    let scale = frame.scale();
    let fresh =
        app.settings.cache.as_ref().is_some_and(|c| {
            c.base.covers(frame.bounds(), scale) && c.card.covers(layout.card, scale)
        });
    if !fresh {
        return false;
    }
    draw_motion(app, frame, &layout);
    true
}

/// 绘制动画途中的一帧：恢复底下的窗口、压上淡入淡出的遮罩（一次绘制完成）；卡片当作一个图层叠
/// 上去，与 `⌘P` 面板、输入条用同一套方式（`super::blend_card`、`dialog_curve`）。
fn draw_motion(app: &App, frame: &mut Frame<'_>, layout: &SettingsLayout) {
    let p = app.palette;
    let curve = match app.settings.motion {
        Some((opening, clock)) => crate::app::animation::dialog_curve(opening, clock),
        None => (1.0, 1.0),
    };
    let Some(cache) = &app.settings.cache else {
        return;
    };
    // 卡片半透明时，透过卡片看到的下层也该压暗：遮罩铺满整个窗口。
    frame.with_opacity(curve.0, |frame| frame.restore_tinted(&cache.base, p.scrim));
    super::blend_card(frame, p, (layout.card, &cache.card), RADIUS_CARD, curve);
}

/// 绘制设置卡片：两栏、标题和当前页的内容。
fn draw_dialog(app: &mut App, frame: &mut Frame<'_>, layout: &SettingsLayout) {
    let p = app.palette;

    // 分为两栏：左侧用窗口底色，右侧用编辑区底色，中间不绘制分隔线。阴影和细边另行绘制。
    frame.rounded(layout.card, p.editor_bg, RADIUS_CARD);
    let r = RADIUS_CARD;
    frame.rect_corners(layout.nav, p.frame_bg, [r, 0.0, 0.0, r], layout.card);

    let search_caret = draw_search(app, frame, layout);
    draw_nav(app, frame, layout);
    draw_header(app, frame, layout);

    // 滚动内容被限制在标题下方的区域内：滚到上方的行不会压在标题上，滚到下方的行不会绘制到
    // 卡片之外。
    let caret = if layout.page == SettingsPage::Keys && !layout.searching {
        frame.within(layout.body, |frame| keys::draw(app, frame))
    } else {
        frame.within(layout.body, |frame| draw_body(app, frame, layout))
    };
    let caret = caret.or(search_caret);
    if caret.is_some() {
        app.ime.cursor = caret;
    }
}

/// 绘制分类栏顶部的搜索框，返回其中光标的矩形。
///
/// 样式与「快捷键」页的搜索框相同：编辑区底色，有焦点时外圈为强调色，没有焦点时一圈细边，
/// 左侧是放大镜图标。
fn draw_search(app: &mut App, frame: &mut Frame<'_>, layout: &SettingsLayout) -> Option<Rect> {
    let p = app.palette;
    let search = layout.search;
    let focused = app.focus == Focus::SettingsSearch;
    if focused {
        frame.rounded(search.inset(-1.5, -1.5), p.accent, RADIUS_ROW + 1.5);
    }
    frame.rounded(search, p.editor_bg, RADIUS_ROW);
    if !focused {
        keys::outline(frame, p.border, search);
    }
    let cy = search.y + search.h / 2.0;
    icons::magnifier(frame, (search.x + 14.0, cy), p.text_dim, search);
    let x = search.x + 26.0;
    let area = Rect::new(x, search.y, (search.right() - 8.0 - x).max(0.0), search.h);
    let placeholder = t!("settings-search-placeholder");
    let mut field = std::mem::take(&mut app.settings.search);
    let caret = crate::view::draw_bar_field(app, frame, area, &mut field, placeholder, focused);
    app.settings.search = field;
    caret
}

/// 绘制分类栏：每项一个图标加名称，行样式与文件树的行相同：当前页铺选中底色，悬停的项铺淡底色。
fn draw_nav(app: &App, frame: &mut Frame<'_>, layout: &SettingsLayout) {
    for &(page, row) in &layout.pages {
        // 「Git」「终端」两项随各自的模块淡入淡出（`App::track_modules`），高度在布局中按显示比例收放；
        // 图标和文字仍按整行的高度居中，不随高度挤压。
        let opacity = match page {
            SettingsPage::Terminal => app.module_fx.terminal.opacity(app.terminal_enabled()),
            SettingsPage::Git => app.module_fx.git.opacity(app.git_enabled()),
            _ => 1.0,
        };
        let full = Rect::new(row.x, row.y, row.w, SETTINGS_NAV_ROW);
        frame.with_opacity(opacity, |frame| {
            draw_nav_row(app, frame, layout, page, row, full);
        });
    }
}

/// 绘制分类栏中的一项：`row` 是该项当前占的区域（底色），`full` 是整行高度的区域（图标、文字）。
fn draw_nav_row(
    app: &App,
    frame: &mut Frame<'_>,
    layout: &SettingsLayout,
    page: SettingsPage,
    row: Rect,
    full: Rect,
) {
    let p = app.palette;
    // 搜索时正文是各页的结果，不标出当前页。
    let current = page == layout.page && !layout.searching;
    let hovered = app.settings.hover == Some(SettingsPart::Page(page));
    // 当前分类使用与顶栏视图切换相同的选中样式：凸起底色（深色配色亮一档）加一圈细边；
    // 悬停只铺浅色。两者仅靠底色深浅区分时难以辨认当前页，因此选中项额外加细边。
    if current {
        let (raised, edge) = super::workspace::panel::segment_colors(p, p.sidebar_bg);
        frame.rect_clipped(row, edge, RADIUS_ROW, layout.nav);
        frame.rect_clipped(row.inset(0.5, 0.5), raised, RADIUS_ROW - 0.5, layout.nav);
    } else if hovered {
        frame.rect_clipped(row, p.hover, RADIUS_ROW, layout.nav);
    }

    let icon = (full.x + 16.0, full.y + full.h / 2.0);
    match page {
        SettingsPage::Appearance => icons::appearance(frame, icon, p.text_muted, layout.nav),
        SettingsPage::Editor => icons::code(frame, icon, p.text_muted, layout.nav),
        SettingsPage::Format => icons::format(frame, icon, p.text_muted, layout.nav),
        SettingsPage::Git => icons::git(frame, icon, 1.0, p.text_muted, layout.nav),
        SettingsPage::Terminal => icons::console(frame, icon, p.text_muted, layout.nav),
        SettingsPage::Modules => icons::modules(frame, icon, p.text_muted, layout.nav),
        SettingsPage::Keys => icons::keyboard(frame, icon, p.text_muted, layout.nav),
        SettingsPage::About => icons::about(frame, icon, p.text_muted, layout.nav),
    }
    let color = if current { p.text } else { p.text_muted };
    ui_text(
        frame,
        app,
        full.x + 32.0,
        full,
        page.title(),
        color,
        layout.nav,
    );
}

/// 绘制标题「设置 › 外观」和右上角的关闭按钮；标题各段之间用箭头隔开，与状态栏中的路径样式相同。
///
/// 标题下方不画分隔线：内容滚到标题下面时直接被裁去。
fn draw_header(app: &App, frame: &mut Frame<'_>, layout: &SettingsLayout) {
    let p = app.palette;
    let header = layout.header;
    let x = header.x + SETTINGS_PAD;
    let title = t!("settings-title");
    let x = ui_text(frame, app, x, header, title, p.text_muted, header);
    let arrow = (x + 10.0, header.y + header.h / 2.0);
    icons::chevron(frame, arrow, 6.5, Direction::Right, p.text_dim, header);
    let page = if layout.searching {
        t!("settings-search-results")
    } else {
        layout.page.title()
    };
    ui_text(frame, app, x + 20.0, header, page, p.text, header);

    let close = layout.close;
    let hovered = app.settings.hover == Some(SettingsPart::Close);
    draw_button(frame, p, close, layout.card, hovered, false);
    icons::close(frame, center(close), 7.0, p.text_muted, close);
}

/// 绘制标题下方的各组设置：组上方是小标题，每组一块圆角底色，行与行之间一条细线。
///
/// 返回输入框内光标的矩形（供输入法候选框定位）。
fn draw_body(app: &mut App, frame: &mut Frame<'_>, layout: &SettingsLayout) -> Option<Rect> {
    let p = app.palette;
    let body = layout.body;

    // 没有匹配的设置项：在正文顶部说明，与组内名称左对齐。
    if layout.searching && layout.rows.is_empty() {
        let x = body.x + SETTINGS_PAD + SETTINGS_INSET;
        let line = Rect::new(x, body.y, body.w, SETTINGS_ROW);
        let text = t!("settings-search-empty");
        ui_text(frame, app, x, line, text, p.text_muted, body);
        return None;
    }

    // 组标题使用小字号和淡色，避免与设置项争夺视觉重心。
    for &(rect, title) in &layout.headings {
        ui_text_small(frame, app, rect.x, rect, title, p.text_dim, body);
    }
    for &group in &layout.groups {
        frame.rect_clipped(group, p.group_bg, RADIUS_BAR, body);
    }
    for &line in &layout.separators {
        frame.rect_clipped(line, p.separator, 0.0, body);
    }

    let mut caret = None;
    for row in &layout.rows {
        if row.row.bottom() < body.y || row.row.y > body.bottom() {
            continue;
        }
        caret = caret.or(draw_row(app, frame, row, body));
    }

    // 窗口太矮、内容放不下时，右侧有一条细滚动条，滚动时或鼠标移到右边缘时显示。
    let page = app.settings.selected.page() as u64;
    let thumb = settings_thumb(body, app.settings.scroll, layout.max_scroll());
    let look = app.scrollbar_look(
        ScrollArea::Settings,
        page,
        app.settings.scroll,
        thumb.map(|_| body),
    );
    if let Some(thumb) = thumb {
        let color = look.color(p.scrollbar, p.scrollbar_active);
        frame.rounded(thumb, color, 2.0);
    }

    caret
}

/// 绘制一行设置：名称在左，控件靠右；较高的行在名称下方还有一行说明。
///
/// 用键盘移动时，选中的行铺一层圆角底色。分组下方的文字按钮（「添加语言…」）前有一个加号，
/// 使用强调色，悬停时铺淡底色。返回正在编辑的输入框的光标矩形。
fn draw_row(app: &mut App, frame: &mut Frame<'_>, row: &SettingRow, clip: Rect) -> Option<Rect> {
    let p = app.palette;
    let selected = app.settings.keyboard
        && app.settings.selected == row.setting
        && matches!(app.focus, Focus::Settings | Focus::Setting);
    let action = row.setting.control() == SettingControl::Action;
    let hovered = app.settings.hover == Some(SettingsPart::Action(row.setting));
    if action && (hovered || selected) {
        let color = if selected {
            p.selected_inactive
        } else {
            p.hover
        };
        frame.rect_clipped(row.row, color, RADIUS_ROW, clip);
    } else if selected {
        // 内缩 4 逻辑像素，使圆角在组的底色上清晰可见。
        frame.rect_clipped(
            row.row.inset(4.0, 4.0),
            p.selected_inactive,
            RADIUS_ROW,
            clip,
        );
    }

    if action {
        let icon = (row.row.x + 14.0, icon_center_y(app, row.row, app.fonts.ui));
        icons::plus(frame, icon, 9.0, p.accent, clip);
        let text = row.setting.label();
        ui_text(frame, app, row.row.x + 26.0, row.row, text, p.accent, clip);
        return None;
    }

    let x = row.row.x + SETTINGS_INSET;
    let room = crate::layout::description_room(row.row, row.control);
    let editing = matches!(&app.settings.editing, Some((s, _)) if *s == row.setting);
    // 字体名无法识别时，用红色小字写在输入框左侧。
    let error = match &app.settings.error {
        Some((setting, error)) if *setting == row.setting => Some(error.clone()),
        _ => None,
    };
    let tall = row.row.h >= SETTINGS_ROW_TALL;
    match row.setting.control() {
        // 主题的缩略图排满整行，名称是上面的组标题。
        SettingControl::ThemeList | SettingControl::Product => {}
        _ if tall => {
            let (top, bottom) = two_lines(row.row);
            ui_text(frame, app, x, top, row.setting.label(), p.text, clip);
            let bottom = Rect::new(x, bottom.y, room, bottom.h);
            match row.setting {
                // 正在编辑命令时，原有的状态和警告不再显示，改为提示如何确认。
                Setting::FormatCommand(_) | Setting::FixCommand(_) if editing => {
                    let hint = t!("settings-command-edit-hint");
                    let hint = ellipsize(frame, hint, app.fonts.ui_small, room);
                    ui_text_small(frame, app, x, bottom, &hint, p.text_muted, clip);
                }
                Setting::FormatCommand(_) | Setting::FixCommand(_) => {
                    draw_tool_detail(app, frame, row.setting, bottom)
                }
                // Git 模块开着但用不了：说明换成原因。
                Setting::GitModule
                    if app.git_enabled()
                        && let Some(problem) = &app.git.problem =>
                {
                    let cy = bottom.y + bottom.h / 2.0;
                    icons::warning(frame, (x + 6.0, cy), p.warning, clip);
                    let text = ellipsize(frame, problem, app.fonts.ui_small, room - 18.0);
                    ui_text_small(frame, app, x + 18.0, bottom, &text, p.warning, clip);
                }
                // 更新：说明换成当前的状态，找到新版本时用强调色。
                Setting::Updates => {
                    let (text, color) = update_status(app);
                    let text = ellipsize(frame, &text, app.fonts.ui_small, room);
                    ui_text_small(frame, app, x, bottom, &text, color, clip);
                }
                // 说明按布局时的宽度折行，最后一行仍放不下时在末尾截短。
                setting => {
                    let text = setting.description().unwrap_or_default();
                    let size = app.fonts.ui_small;
                    let mut measure = |s: &str| frame.measure(FontKind::Ui, size, s);
                    let lines =
                        crate::layout::description_lines(setting, &text, room, &mut measure);
                    let last = lines.len().saturating_sub(1);
                    for (i, line) in lines.into_iter().enumerate() {
                        let line = if i == last {
                            truncate_end(frame, line, size, room)
                        } else {
                            Cow::Borrowed(line)
                        };
                        let at =
                            Rect::new(x, bottom.y + i as f32 * SETTINGS_DESC_LINE, room, bottom.h);
                        ui_text_small(frame, app, x, at, &line, p.text_muted, clip);
                    }
                }
            }
        }
        _ => {
            let end = ui_text(frame, app, x, row.row, row.setting.label(), p.text, clip);
            if let Some(error) = error {
                let room = (row.control.x - 12.0 - end - 16.0).max(0.0);
                let error = ellipsize(frame, &error, app.fonts.ui_small, room);
                let w = frame.measure(FontKind::Ui, app.fonts.ui_small, &error);
                let at = row.control.x - 12.0 - w;
                ui_text_small(frame, app, at, row.row, &error, p.error, clip);
            }
        }
    }

    match row.setting.control() {
        SettingControl::Font => draw_field(app, frame, row, clip),
        SettingControl::Command if editing => draw_field(app, frame, row, clip),
        SettingControl::Command => {
            let (text, dim) = match app.settings.tool_row(row.setting) {
                Some(r) => (r.tool.clone(), r.unset),
                None if row.setting == Setting::TerminalShell => match app.configured_shell() {
                    Some(shell) => (shell.name, false),
                    None => (t!("settings-terminal-shell-default").to_owned(), false),
                },
                None => (t!("format-tool-auto").to_owned(), false),
            };
            let hovered = app.settings.hover == Some(SettingsPart::Field(row.setting));
            draw_popup(app, frame, row.control, &text, dim, hovered, clip);
            None
        }
        SettingControl::Choice => {
            let (text, dim) = if row.setting == Setting::Language {
                let choice = app.config.language.unwrap_or_default();
                (crate::layout::locale_label(choice).to_owned(), false)
            } else if row.setting == Setting::RenderWhitespace {
                let choice = app.config.render_whitespace.unwrap_or_default();
                (crate::layout::whitespace_label(choice).to_owned(), false)
            } else if row.setting == Setting::VimClipboard {
                let choice = app.config.vim_clipboard.unwrap_or_default();
                (crate::layout::vim_clipboard_label(choice).to_owned(), false)
            } else if row.setting == Setting::Zoom {
                (format!("{}%", app.config.ui_zoom.unwrap_or(100)), false)
            } else if row.setting == Setting::BackgroundFit {
                let choice = app.config.ui_background_fit.unwrap_or_default();
                (
                    crate::layout::background_fit_label(choice).to_owned(),
                    false,
                )
            } else if row.setting == Setting::BackgroundImage {
                let name = (app.config.ui_background_image.as_deref())
                    .map(|path| winc_editor::display_name(std::path::Path::new(path)).into_owned());
                let none = name.is_none();
                (
                    name.unwrap_or_else(|| t!("settings-background-none").to_owned()),
                    none,
                )
            } else if row.setting == Setting::TerminalPosition {
                let text = if app.terminal_right() {
                    t!("settings-terminal-position-right")
                } else {
                    t!("settings-terminal-position-bottom")
                };
                (text.to_owned(), false)
            } else if row.setting == Setting::TerminalOptionAsMeta {
                let choice = (app.config.option_as_meta).unwrap_or(winc_config::OptionAsMeta::Left);
                (
                    crate::layout::option_as_meta_label(choice).to_owned(),
                    false,
                )
            } else {
                let none = || t!("settings-skip-on-save-none").to_owned();
                (app.skipped_on_save().unwrap_or_else(none), false)
            };
            draw_popup(app, frame, row.control, &text, dim, hovered, clip);
            None
        }
        SettingControl::Switch => {
            draw_switch(app, frame, row, clip);
            None
        }
        SettingControl::Action | SettingControl::Search => None,
        SettingControl::Link => {
            draw_link(app, frame, row, clip);
            None
        }
        SettingControl::Product => {
            draw_product(app, frame, row, clip);
            None
        }
        SettingControl::Button => {
            // 不支持应用内更新时说明行已写明原因，不显示按钮；正在检查、下载时按钮变淡。
            if app.updates_supported() {
                let (label, role, enabled) = update_button(app);
                draw_action_button(app, frame, row.control, &label, role, enabled, hovered);
            }
            None
        }
        SettingControl::Stepper => {
            draw_stepper(app, frame, row, clip);
            None
        }
        SettingControl::Themes => {
            draw_mode(app, frame, row, clip);
            None
        }
        SettingControl::ThemeList => {
            draw_theme_grid(app, frame, row, clip);
            None
        }
    }
}

/// 绘制「关于」页的链接：右对齐的显示名后跟外链箭头。
///
/// 悬停时只在内容处铺悬停底色（[`crate::layout::value_area`]），点击范围与之相同。
fn draw_link(app: &App, frame: &mut Frame<'_>, row: &SettingRow, clip: Rect) {
    let p = app.palette;
    let text = crate::layout::about_value(row.setting).unwrap_or_default();
    let text_w = frame.measure(FontKind::Ui, app.fonts.ui, &text);
    let icon_w = crate::layout::LINK_ICON_W;
    let area = crate::layout::value_area(row.control, text_w + icon_w);
    if app.settings.hover == Some(SettingsPart::Link(row.setting)) {
        frame.rect_clipped(area, p.hover, RADIUS_ROW, clip);
    }
    let right = area.right() - SETTINGS_GLYPH_PAD;
    let center = (right - 4.5, area.y + area.h / 2.0);
    icons::external(frame, center, p.text_dim, clip);
    ui_text(
        frame,
        app,
        right - icon_w - text_w,
        area,
        &text,
        p.text_muted,
        clip,
    );
}

/// 绘制「关于」页顶部的产品信息：应用图标、名称和版本号，水平居中（[`ProductParts`]）。
///
/// 图标按当前缩放从应用图标的 SVG 渲染，与屏幕像素一一对应。版本号点击复制，悬停时只在文字处
/// 铺悬停底色。
fn draw_product(app: &mut App, frame: &mut Frame<'_>, row: &SettingRow, clip: Rect) {
    let p = app.palette;
    let parts = ProductParts::new(row.row);
    let size = (SETTINGS_LOGO * frame.scale()).round() as u32;
    let logo = parts.logo;
    frame.pixels((logo.x, logo.y), size, size, app.product_logo(size), clip);

    // 名称比界面文字大 7 点，随界面字号一同缩放。
    let name_size = app.fonts.ui + 7.0;
    let name = crate::layout::PRODUCT_NAME;
    let w = frame.measure(FontKind::Ui, name_size, name);
    let x = (parts.name.x + (parts.name.w - w) / 2.0).round();
    label(frame, app, name_size, (x, parts.name), name, p.text, clip);

    let text = crate::layout::version_line();
    let w = frame.measure(FontKind::Ui, app.fonts.ui, &text);
    let area = parts.version_area(w);
    if app.settings.hover == Some(SettingsPart::Copy(row.setting)) {
        frame.rect_clipped(area, p.hover, RADIUS_ROW, clip);
    }
    let x = (area.x + (area.w - w) / 2.0).round();
    ui_text(frame, app, x, area, &text, p.text_muted, clip);
}

/// 「关于」页更新按钮上的文字、样式，以及现在能不能点。
///
/// 还没找到新版本时是「检查更新」。找到之后与状态栏相同：下载安装、下载中显示百分比、
/// 准备好之后重新启动；安装失败时是「重试」。正在检查或下载时按钮变淡，点了没有反应。
fn update_button(app: &App) -> (String, ButtonRole, bool) {
    use crate::app::UpdateClick;
    use crate::winc::UpdateState;
    let state = app.shared.update_state.borrow();
    let enabled = UpdateClick::from_state(&state) != UpdateClick::None;
    let (label, role) = match &*state {
        UpdateState::Downloading(_, percent) => (format!("{percent}%"), ButtonRole::Normal),
        UpdateState::Ready(_) => (
            t!("settings-update-restart").to_owned(),
            ButtonRole::Primary,
        ),
        UpdateState::InstallFailed(_) => {
            (t!("settings-update-retry").to_owned(), ButtonRole::Normal)
        }
        UpdateState::Found(_) => (
            t!("settings-update-download").to_owned(),
            ButtonRole::Primary,
        ),
        _ => (t!("settings-check-now").to_owned(), ButtonRole::Normal),
    };
    (label, role, enabled)
}

/// 「关于」页「更新」一行下方的状态文字和颜色；也是状态栏更新按钮的悬停提示（见
/// `status::update_tooltip`）。
pub(in crate::view) fn update_status(app: &App) -> (String, Color) {
    use crate::winc::UpdateState;
    let p = app.palette;
    match &*app.shared.update_state.borrow() {
        UpdateState::Off => (t!("settings-updates-off").to_owned(), p.text_muted),
        UpdateState::Idle => (t!("settings-updates-idle").to_owned(), p.text_muted),
        UpdateState::Checking => (t!("settings-updates-checking").to_owned(), p.text_muted),
        UpdateState::Latest => (t!("settings-updates-latest").to_owned(), p.text_muted),
        UpdateState::Found(version) => {
            let version = crate::release_version::present_version(version);
            (
                t!("settings-updates-found", version = version.as_str()),
                p.accent,
            )
        }
        UpdateState::Failed => (t!("settings-updates-failed").to_owned(), p.text_muted),
        UpdateState::Downloading(version, percent) => {
            let version = crate::release_version::present_version(version);
            (
                t!(
                    "settings-updates-downloading",
                    version = version.as_str(),
                    percent = *percent
                ),
                p.text_muted,
            )
        }
        UpdateState::Ready(version) => {
            let version = crate::release_version::present_version(version);
            (
                t!("settings-updates-ready", version = version.as_str()),
                p.accent,
            )
        }
        UpdateState::InstallFailed(error) => (
            t!("settings-updates-install-failed", error = error.as_str()),
            p.error,
        ),
    }
}

/// 返回两行高的设置行中，名称所在的行和下方说明所在的行。
fn two_lines(row: Rect) -> (Rect, Rect) {
    let top = Rect::new(row.x, row.y + 5.0, row.w, 26.0);
    let bottom = Rect::new(row.x, top.bottom() - 2.0, row.w, 20.0);
    (top, bottom)
}

/// 绘制一种语言名称下方的说明行：工具是如何确定的（指定了命令时为命令本身），其后是警告，
/// 可安装的工具再跟一个「安装方法…」链接。
///
/// 空间不足时先截短前面的说明文本。
fn draw_tool_detail(app: &mut App, frame: &mut Frame<'_>, setting: Setting, line: Rect) {
    let p = app.palette;
    let Some(row) = app.settings.tool_row(setting).cloned() else {
        return;
    };
    let size = app.fonts.ui_small;
    let link = t!("format-install-link");
    // 说明、警告、链接之间的间距（逻辑像素）。
    const GAP: f32 = 10.0;

    let link_w = if row.install.is_empty() {
        0.0
    } else {
        GAP + frame.measure(FontKind::Ui, size, link)
    };
    let warning_w = row
        .warning
        .as_ref()
        .map_or(0.0, |w| GAP + 16.0 + frame.measure(FontKind::Ui, size, w));
    let room = (line.w - warning_w - link_w).max(0.0);
    let detail = ellipsize(frame, &row.detail, size, room);
    let mut x = ui_text_small(frame, app, line.x, line, &detail, p.text_muted, line);

    if let Some(warning) = &row.warning {
        x += GAP;
        let icon = (x + 6.0, icon_center_y(app, line, size) - 0.5);
        icons::warning(frame, icon, p.warning, line);
        x = ui_text_small(frame, app, x + 16.0, line, warning, p.warning, line);
    }
    if !row.install.is_empty() {
        x += GAP;
        let end = ui_text_small(frame, app, x, line, link, p.accent, line);
        let hit = Rect::new(x - 2.0, line.y, end - x + 4.0, line.h);
        if app.settings.hover == Some(SettingsPart::Install(setting)) {
            let underline = Rect::new(x, (line.bottom() - 4.0).round(), end - x, 1.0);
            frame.rect_clipped(underline, p.accent, 0.0, line);
        }
        app.hits.install_links.push((setting, hit));
    }
}

/// 绘制点击后弹出菜单的值：平时是一段右对齐的淡色文本（未选择时 `dim` 为真，颜色更淡），
/// 其后一对上下小箭头表示可选；悬停时铺悬停底色。
///
/// 不绘制边框，样式与字体名等其他设置项一致。
fn draw_popup(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    text: &str,
    dim: bool,
    hovered: bool,
    clip: Rect,
) {
    let p = app.palette;
    if hovered {
        frame.rect_clipped(rect, p.hover, RADIUS_ROW, clip);
    }
    let area = Rect::new(
        rect.x + 10.0,
        rect.y,
        (rect.w - 10.0 - SETTINGS_GLYPH_PAD).max(0.0),
        rect.h,
    );
    let arrows = (area.right() - 3.0, rect.y + rect.h / 2.0);
    icons::up_down(frame, arrows, p.text_dim, clip);

    let room = Rect::new(area.x, area.y, (area.w - 16.0).max(0.0), area.h);
    let text = ellipsize(frame, text, app.fonts.ui, room.w);
    let w = frame.measure(FontKind::Ui, app.fonts.ui, &text);
    let color = if dim { p.text_dim } else { p.text_muted };
    ui_text(frame, app, room.right() - w, room, &text, color, room);
}

/// 绘制字体名和正在输入的格式化命令：平时是右对齐的一段文本（未设置时显示「系统默认」），
/// 悬停时铺输入框底色，点击后变成输入框，外圈为强调色（字体名无效时为错误色）。
///
/// 正在编辑时返回光标矩形，否则返回 `None`。
fn draw_field(app: &mut App, frame: &mut Frame<'_>, row: &SettingRow, clip: Rect) -> Option<Rect> {
    let p = app.palette;
    let rect = row.control;
    let editing = matches!(&app.settings.editing, Some((s, _)) if *s == row.setting);
    let hovered = app.settings.hover == Some(SettingsPart::Field(row.setting));
    let font = row.setting.control() == SettingControl::Font;
    let on_menu = font && app.settings.hover == Some(SettingsPart::FontMenu(row.setting));
    let hovered = hovered || on_menu;
    let (configured, placeholder) = match row.setting {
        Setting::UiFont => (app.config.ui_font.clone(), t!("settings-font-default")),
        Setting::TerminalFont => (
            app.config.terminal_font.clone(),
            t!("settings-terminal-font-default"),
        ),
        Setting::FormatCommand(_) | Setting::FixCommand(_) => {
            (None, t!("settings-command-placeholder"))
        }
        Setting::TerminalShell => (None, t!("settings-terminal-shell-placeholder")),
        _ => (
            app.config.editor_font.clone(),
            winc_render::DEFAULT_EDITOR_FONT,
        ),
    };
    // 字体输入框的文字不进入右端的下拉按钮。
    let area = if font {
        let button = crate::layout::font_menu_button(rect);
        Rect::new(
            rect.x + 10.0,
            rect.y,
            (button.x - rect.x - 10.0).max(0.0),
            rect.h,
        )
    } else {
        rect.inset(10.0, 0.0)
    };
    let draw_arrows = |app: &App, frame: &mut Frame<'_>| {
        if font {
            let arrows = (
                rect.right() - SETTINGS_GLYPH_PAD - 3.0,
                rect.y + rect.h / 2.0,
            );
            icons::up_down(frame, arrows, app.palette.text_dim, clip);
        }
    };

    if editing {
        let ring = if matches!(&app.settings.error, Some((s, _)) if *s == row.setting) {
            p.error
        } else {
            p.accent
        };
        frame.rect_clipped(rect.inset(-1.5, -1.5), ring, RADIUS_ROW + 1.5, clip);
        frame.rect_clipped(rect, p.editor_bg, RADIUS_ROW, clip);

        let focused = app.focus == Focus::Setting;
        let (setting, mut field) = app.settings.editing.take().expect("刚刚判断过在编辑");
        let caret = draw_bar_field(app, frame, area, &mut field, placeholder, focused);
        app.settings.editing = Some((setting, field));
        if on_menu {
            let button = crate::layout::font_menu_button(rect);
            frame.rect_clipped(button, p.hover, RADIUS_ROW, clip);
        }
        draw_arrows(app, frame);
        return caret;
    }

    if hovered {
        frame.rect_clipped(rect, p.hover, RADIUS_ROW, clip);
    }

    // 未设置时显示的「系统默认」「跟随编辑器」是菜单中的一项，与其他取值同色。
    let text = configured.map_or(Cow::Borrowed(placeholder), Cow::Owned);
    let color = p.text_muted;
    let text = ellipsize(frame, &text, app.fonts.ui, area.w);
    let w = frame.measure(FontKind::Ui, app.fonts.ui, &text);
    ui_text(frame, app, area.right() - w, area, &text, color, area);
    draw_arrows(app, frame);
    None
}

/// 绘制开关：圆角底色，开启时为强调色、圆钮在右侧，关闭时为灰色、圆钮在左侧。
///
/// 刚切换时圆钮滑到另一侧，强调色随之淡入或淡出。
fn draw_switch(app: &App, frame: &mut Frame<'_>, row: &SettingRow, clip: Rect) {
    let p = app.palette;
    let track = row.control;
    let on = if app.switch_on(row.setting) { 1.0 } else { 0.0 };
    // 开启程度：0 为关闭，1 为开启，切换途中从原状态过渡到新状态。
    let t = match app.settings.switch_slide {
        Some((setting, clock)) if setting == row.setting => clock.between(1.0 - on, on),
        _ => on,
    };
    let radius = track.h / 2.0;
    frame.rect_clipped(track, p.switch_off, radius, clip);
    if t > 0.0 {
        let accent = Color {
            a: p.accent.a * t,
            ..p.accent
        };
        frame.rect_clipped(track, accent, radius, clip);
    }
    let knob = track.h - 4.0;
    let left = track.x + 2.0;
    let x = left + (track.right() - 2.0 - knob - left) * t;
    let knob = Rect::new(x, track.y + 2.0, knob, knob);
    frame.rect_clipped(knob, p.switch_knob, knob.h / 2.0, clip);
}

/// 绘制数值步进控件：「− 13 ＋」。
///
/// 按钮平时没有底色，悬停时才有；到达可调范围的上下限时，对应一侧的按钮变淡。
fn draw_stepper(app: &App, frame: &mut Frame<'_>, row: &SettingRow, clip: Rect) {
    let p = app.palette;
    let parts = StepperParts::new(row.control);
    let (value, at_min, at_max) = app.setting_value(row.setting);

    for (button, part, enabled) in [
        (parts.minus, SettingsPart::Decrease(row.setting), !at_min),
        (parts.plus, SettingsPart::Increase(row.setting), !at_max),
    ] {
        let hovered = enabled && app.settings.hover == Some(part);
        draw_button(frame, p, button, clip, hovered, false);

        let color = if enabled {
            p.text_muted
        } else {
            p.text_dim.alpha(p.text_dim.a * 0.5)
        };
        let c = center(button);
        match part {
            SettingsPart::Decrease(_) => icons::minus(frame, c, 8.0, color, clip),
            _ => icons::plus(frame, c, 8.0, color, clip),
        }
    }

    let w = frame.measure(FontKind::Ui, app.fonts.ui, &value);
    let x = (parts.value.x + (parts.value.w - w) / 2.0).round();
    caps_label(
        frame,
        app,
        app.fonts.ui,
        (x, parts.value),
        &value,
        p.text,
        clip,
    );
}

/// 绘制「模式」：跟随系统、浅色、深色三段的分段控件，样式与顶栏的视图切换相同——一条底槽，
/// 选中的那一段凸起，悬停的那一段铺浅色。
fn draw_mode(app: &App, frame: &mut Frame<'_>, row: &SettingRow, clip: Rect) {
    let p = app.palette;
    let track = row.control;
    frame.rect_clipped(track, p.hover, RADIUS_ROW + 2.0, clip);
    let track_bg = super::workspace::panel::over(p.group_bg, p.hover);
    let (raised, edge) = super::workspace::panel::segment_colors(p, track_bg);
    // 凸起的那一段：换了模式时从原来那一段滑过来（宽度也跟着变），先绘制，字压在上面。
    let index = |choice: ThemeChoice| THEME_CHOICES.iter().position(|c| c.0 == choice);
    if let Some(to) = index(app.theme_choice).map(|i| theme_segment(track, i)) {
        let slot = match app.settings.mode_slide {
            Some((from, clock)) => match index(from).map(|i| theme_segment(track, i)) {
                Some(from) => Rect::new(
                    clock.between(from.x, to.x),
                    to.y,
                    clock.between(from.w, to.w),
                    to.h,
                ),
                None => to,
            },
            None => to,
        };
        frame.rect_clipped(slot, edge, RADIUS_ROW, clip);
        frame.rect_clipped(slot.inset(0.5, 0.5), raised, RADIUS_ROW - 0.5, clip);
    }
    for (i, &(choice, label)) in THEME_CHOICES.iter().enumerate() {
        let label = label();
        let segment = theme_segment(track, i);
        let selected = app.theme_choice == choice;
        let hovered = app.settings.hover == Some(SettingsPart::Theme(choice));
        if hovered && !selected {
            frame.rect_clipped(segment, p.hover, RADIUS_ROW, clip);
        }
        let color = if selected || hovered {
            p.text
        } else {
            p.text_muted
        };
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, label);
        let x = (segment.x + (segment.w - w) / 2.0).round();
        ui_text_small(frame, app, x, segment, label, color, clip);
    }
}

/// 绘制浅色或深色主题的格子：每个主题一张小窗口缩略图（用它自己的配色绘制），下方是名称。
///
/// 选定的格子外圈是 2 像素的强调色，和缩略图之间隔一圈组底色；悬停的外圈是一道淡边。
fn draw_theme_grid(app: &App, frame: &mut Frame<'_>, row: &SettingRow, clip: Rect) {
    let p = app.palette;
    let dark = row.setting == Setting::DarkTheme;
    let chosen = app.theme_slot(dark);
    for (i, theme) in themes(dark).iter().enumerate() {
        let tile = theme_tile(row.control, i);
        let thumb = theme_thumb(tile);
        let selected = chosen == i;
        let hovered = app.settings.hover == Some(SettingsPart::NamedTheme(dark, i));
        let ring = if selected {
            Some(p.accent)
        } else if hovered {
            Some(p.text_dim)
        } else {
            None
        };
        if let Some(ring) = ring {
            frame.rect_clipped(thumb.inset(-3.0, -3.0), ring, THUMB_RADIUS + 3.0, clip);
            frame.rect_clipped(
                thumb.inset(-1.0, -1.0),
                p.group_bg,
                THUMB_RADIUS + 1.0,
                clip,
            );
        }
        mini_window(frame, theme.palette, thumb, clip);
        // 缩略图自带一圈很淡的边，和组底色相近的缩略图（深色放在深色上）也有清楚的轮廓。
        outline(frame, p.popup_border, thumb, THUMB_RADIUS, clip);

        let name = Rect::new(tile.x, thumb.bottom() + 4.0, tile.w, tile.h - thumb.h - 4.0);
        let color = if selected { p.text } else { p.text_muted };
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, theme.name());
        let x = (name.x + (name.w - w) / 2.0).round();
        ui_text_small(frame, app, x, name, theme.name(), color, clip);
    }
}

/// 主题缩略图的圆角半径（逻辑像素）。
const THUMB_RADIUS: f32 = 6.0;

/// 绘制一个示意用的小窗口，用来预览一套配色：按主窗口的布局等比缩小，窗框底色上左边一张
/// 侧栏卡片、右边一张编辑区卡片；侧栏是几行文件名（一行选中），编辑区顶上一个当前标签，
/// 下面几行行号和带语法颜色的代码，其中一行是当前行、一行是新增的（铺差异的底色）。
fn mini_window(frame: &mut Frame<'_>, p: &Palette, rect: Rect, clip: Rect) {
    // 一行文字的粗细和行距按缩略图的高度定：缩略图按 66 逻辑像素高设计，`unit` 是一个设计单位
    // 的高度。
    let unit = rect.h / 66.0;
    let bar = (2.0 * unit).max(1.5);
    let pitch = 6.5 * unit;
    let pad = (4.0 * unit).round();
    let radius = 3.0 * unit;
    let line = |frame: &mut Frame<'_>, x: f32, y: f32, w: f32, color: Color| {
        frame.rect_clipped(Rect::new(x, y, w, bar), color, bar / 2.0, clip);
    };

    frame.rect_clipped(rect, p.frame_bg, THUMB_RADIUS, clip);
    let top = (8.0 * unit).round();
    let inner = Rect::new(
        rect.x + pad,
        rect.y + top,
        rect.w - pad * 2.0,
        rect.h - top - pad,
    );
    let side_w = (inner.w * 0.3).round();
    let side = Rect::new(inner.x, inner.y, side_w, inner.h);
    let editor = Rect::new(
        side.right() + pad,
        inner.y,
        inner.right() - side.right() - pad,
        inner.h,
    );
    frame.rect_clipped(side, p.sidebar_bg, radius, clip);
    frame.rect_clipped(editor, p.editor_bg, radius, clip);

    // 窗框顶上左边三个小圆点（窗口按钮的位置），用最淡的字色。
    for i in 0..3 {
        let d = 2.5 * unit;
        let dot = Rect::new(
            rect.x + pad + 1.0 + i as f32 * 4.5 * unit,
            rect.y + 3.0 * unit,
            d,
            d,
        );
        frame.rect_clipped(dot, p.text_dim.alpha(0.6), d / 2.0, clip);
    }

    // 文件树：第三行是选中的行，一行是改动的文件（Git 的修改色）。
    let rows: [(f32, Color); 6] = [
        (0.55, p.text_muted),
        (0.7, p.text_muted),
        (0.6, p.text),
        (0.75, p.git_modified),
        (0.5, p.text_muted),
        (0.65, p.git_added),
    ];
    for (i, (w, color)) in rows.into_iter().enumerate() {
        let y = side.y + 5.0 * unit + i as f32 * pitch;
        if i == 2 {
            let back = Rect::new(side.x + 2.0, y - 2.0 * unit, side.w - 4.0, bar + 4.0 * unit);
            frame.rect_clipped(back, p.selected, 1.5 * unit, clip);
        }
        let indent = if i == 0 { 0.0 } else { 3.0 * unit };
        line(
            frame,
            side.x + 4.0 * unit + indent,
            y,
            (side.w - 10.0 * unit) * w,
            color,
        );
    }

    // 标签栏：一个当前标签（主题的底色和细边）。
    let tab = Rect::new(
        editor.x + 3.0 * unit,
        editor.y + 3.0 * unit,
        (editor.w * 0.34).round(),
        7.0 * unit,
    );
    frame.rect_clipped(tab, p.tab_edge, 2.0 * unit, clip);
    frame.rect_clipped(tab.inset(0.5, 0.5), p.tab_fill, 2.0 * unit, clip);
    line(
        frame,
        tab.x + 3.0 * unit,
        tab.y + (tab.h - bar) / 2.0,
        tab.w - 6.0 * unit,
        p.text,
    );

    // 代码：行号一列，右边是几行有语法颜色的代码。第三行是当前行，第五行是新增的行。
    let gutter = editor.x + 4.0 * unit;
    let x0 = editor.x + 12.0 * unit;
    let room = editor.right() - 4.0 * unit - x0;
    let code: [(f32, &[(Color, f32)]); 6] = [
        (0.0, &[(p.syntax_comment, 0.55)]),
        (
            0.0,
            &[
                (p.syntax_keyword, 0.14),
                (p.syntax_function, 0.28),
                (p.syntax_text, 0.08),
            ],
        ),
        (
            0.1,
            &[
                (p.syntax_keyword, 0.1),
                (p.syntax_text, 0.12),
                (p.syntax_string, 0.32),
            ],
        ),
        (0.1, &[(p.syntax_property, 0.16), (p.syntax_builtin, 0.2)]),
        (0.1, &[(p.syntax_function, 0.2), (p.syntax_number, 0.08)]),
        (0.0, &[(p.syntax_text, 0.06)]),
    ];
    for (i, (indent, parts)) in code.into_iter().enumerate() {
        let y = tab.bottom() + 5.0 * unit + i as f32 * pitch;
        let band = Rect::new(editor.x, y - 2.0 * unit, editor.w, bar + 4.0 * unit);
        let number = if i == 2 {
            frame.rect_clipped(band, p.current_line, 0.0, clip);
            p.gutter_text_current
        } else if i == 4 {
            frame.rect_clipped(band, p.diff_added.line, 0.0, clip);
            let mark = Rect::new(gutter + 4.0 * unit, band.y, 1.0 * unit, band.h);
            frame.rect_clipped(mark, p.diff_added.mark, 0.0, clip);
            p.gutter_text
        } else {
            p.gutter_text
        };
        line(frame, gutter, y, 3.0 * unit, number);
        let mut x = x0 + indent * room;
        for &(color, w) in parts {
            let w = (w * room).round();
            line(frame, x, y, w, color);
            x += w + 2.0 * unit;
        }
    }
}

/// 绘制圆角矩形外沿 1 逻辑像素的细边。
///
/// 矩形内部已经绘制了内容，不能先铺边色再铺底色，因此用四条直线和四段圆弧描边。
fn outline(frame: &mut Frame<'_>, color: Color, rect: Rect, radius: f32, clip: Rect) {
    let inner = rect.inset(0.5, 0.5);
    let shapes = icons::rounded_outline(inner.x, inner.y, inner.w, inner.h, radius - 0.5);
    frame.stroke(&shapes, 1.0, color, clip);
}
