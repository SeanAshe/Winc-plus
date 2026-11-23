//! 开始页：没有打开目录时，编辑区内显示的应用名称、开始操作和最近目录。首次设置尚未完成时
//! 只显示「首次设置」：语言、外观、主题和可选模块，点击后立即生效，点击「完成」后换回开始操作
//! 和最近目录。

use winc_i18n::t;
use winc_render::{Frame, Rect};

use super::*;
use crate::app::StartAction;

/// 内容列的宽度（逻辑像素）。
const WIDTH: f32 = 420.0;
/// 一行的高度（逻辑像素）。
const ROW: f32 = 32.0;
/// 小标题的高度（逻辑像素）。
const SECTION_TITLE: f32 = 28.0;
/// 两段之间的空隙（逻辑像素）。
const GAP: f32 = 20.0;
/// 「首次设置」的行数：外观、主题、语言和三个模块。
const SETUP_ROWS: f32 = 6.0;
/// 「首次设置」中一行的高度（逻辑像素）：比开始操作的行高，容纳分段控件。
const SETUP_ROW: f32 = 38.0;
/// 分段控件的高度（逻辑像素），与设置中「模式」的分段控件相同。
const SEGMENTS_H: f32 = 28.0;
/// 分段控件中一段的文字两侧各留的空白（逻辑像素）。
const SEGMENT_PAD: f32 = 14.0;
/// 下拉值的高度（逻辑像素）。
const DROPDOWN_H: f32 = 26.0;
/// 「完成」按钮的高度（逻辑像素）。
const SETUP_BUTTON_H: f32 = 28.0;
/// 启动动画中窗口中央应用图标的边长（逻辑像素）。
const SPLASH_LOGO: f32 = 96.0;
/// 结束首次设置的过渡中，开始页淡入时自下而上移动的距离（逻辑像素）。
const OUTRO_RISE: f32 = 10.0;
/// 启动动画中内容淡入时自下而上移动的距离（逻辑像素）。
const SETUP_RISE: f32 = 8.0;
/// 最近列表最多显示的目录数（个）。
const MAX_SHOWN: usize = 8;

/// 「开始」一段的操作：操作、用于显示快捷键的键位表动作；行上的文字见 [`action_label`]。
///
/// 只有 macOS 的选择器能同时选文件和目录，其他平台上打开文件和打开目录是两项。
const ACTIONS: &[(StartAction, Action)] = if cfg!(target_os = "macos") {
    &[
        (StartAction::Settings, Action::Settings),
        (StartAction::NewFile, Action::NewFile),
        (StartAction::Open, Action::OpenFile),
    ]
} else {
    &[
        (StartAction::Settings, Action::Settings),
        (StartAction::NewFile, Action::NewFile),
        (StartAction::Open, Action::OpenFile),
        (StartAction::OpenFolder, Action::OpenFolder),
    ]
};

/// 返回「开始」一段要显示的操作：[`ACTIONS`]，远程模块开着时加上连接到 SSH 主机。
fn actions(app: &App) -> Vec<(StartAction, Action)> {
    let mut actions = ACTIONS.to_vec();
    if app.remote_enabled() {
        actions.push((StartAction::Remote, Action::RemoteConnect));
    }
    actions
}

/// 返回「开始」一段中操作 `action` 所在行上的文字。
fn action_label(action: StartAction) -> &'static str {
    match action {
        StartAction::Settings => t!("workspace-start-settings"),
        StartAction::NewFile => t!("workspace-start-new-file"),
        StartAction::OpenFolder => t!("workspace-start-open-folder"),
        StartAction::Remote => t!("workspace-start-remote"),
        _ if cfg!(target_os = "macos") => t!("workspace-start-open"),
        _ => t!("workspace-start-open-file"),
    }
}

/// 绘制开始页，并登记各行的点击区域（`start_hits`）。
///
/// 首次设置尚未完成时只显示「首次设置」；在它的两页之间切换，以及刚点击「完成」时，原来的一页
/// 淡出，新的一页随后自下而上淡入，过渡期间淡出的一页不响应点击。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    if app.state.setup_pending {
        let keys = app.setup.shown;
        let Some(t) = app.motion.setup_page.map(|clock| clock.linear()) else {
            draw_setup_page(app, frame, keys, 0.0);
            return;
        };
        let (leaving, entering) = crate::app::setup::outro_opacity(t);
        if leaving > 0.0 {
            frame.with_opacity(leaving, |frame| draw_setup_page(app, frame, !keys, 0.0));
        }
        app.hits.start.clear();
        let rise = (1.0 - entering) * OUTRO_RISE;
        frame.with_opacity(entering, |frame| draw_setup_page(app, frame, keys, rise));
        return;
    }
    let Some(t) = app.motion.setup_outro.map(|clock| clock.linear()) else {
        draw_start(app, frame, 0.0);
        return;
    };
    let (leaving, entering) = crate::app::setup::outro_opacity(t);
    if leaving > 0.0 {
        let keys = app.setup.leaving;
        frame.with_opacity(leaving, |frame| draw_setup_page(app, frame, keys, 0.0));
    }
    app.hits.start.clear();
    let rise = (1.0 - entering) * OUTRO_RISE;
    frame.with_opacity(entering, |frame| draw_start(app, frame, rise));
}

/// 绘制开始操作和最近目录，整体下移 `rise`（逻辑像素），并登记各行的点击区域。
fn draw_start(app: &mut App, frame: &mut Frame<'_>, rise: f32) {
    let p = app.palette;
    let editor = app.layout.editor;
    let recent = app.state.recent.len().min(MAX_SHOWN);

    // 先计算总高度，使整块内容在编辑区内垂直居中，并整体上移 24 逻辑像素
    // （上边距至少 16 逻辑像素）。
    // 标题块的高度（逻辑像素）：应用名 34、版本号 20，再留 2 的间隔。
    let title_h = 56.0;
    let actions = actions(app);
    let actions_h = SECTION_TITLE + actions.len() as f32 * ROW;
    let recent_h = if recent > 0 {
        GAP + SECTION_TITLE + recent as f32 * ROW
    } else {
        0.0
    };
    let total = title_h + GAP + actions_h + recent_h;
    let w = WIDTH.min(editor.w - 48.0).max(0.0);
    let x = (editor.x + (editor.w - w) / 2.0).round();
    let mut y = (editor.y + ((editor.h - total) / 2.0 - 24.0).max(16.0) + rise).round();

    let title = Rect::new(x + 10.0, y, w, 34.0);
    crate::view::label(
        frame,
        app,
        app.fonts.ui + 13.0,
        (title.x, title),
        crate::layout::PRODUCT_NAME,
        p.text,
        editor,
    );
    let version = Rect::new(title.x, title.bottom(), w, 20.0);
    let text = t!(
        "workspace-start-version",
        version = crate::release_version::display_version()
    );
    ui_text_small(frame, app, version.x, version, &text, p.text_dim, editor);
    y += title_h + GAP;

    y = section_title(app, frame, t!("workspace-start-section-start"), x, y, w);
    for (action, key) in actions {
        let label = action_label(action);
        let row = Rect::new(x, y, w, ROW);
        draw_row_bg(app, frame, row, action);
        action_icon(app, frame, action, row);
        ui_text(frame, app, row.x + 36.0, row, label, p.text, editor);
        if let Some(key) = keymap::shortcut(key) {
            draw_keycap(app, frame, row.inset(6.0, 0.0), &key, editor);
        }
        app.hits.start.push((row, action));
        y += ROW;
    }

    if recent == 0 {
        return;
    }

    y += GAP;
    y = section_title(app, frame, t!("workspace-start-section-recent"), x, y, w);

    let parents = crate::app::parents(&app.state.recent);
    for index in 0..recent {
        let row = Rect::new(x, y, w, ROW);
        let open = StartAction::Recent(index);
        let forget = StartAction::Forget(index);
        let hovered = matches!(app.pointer.hover, Hover::Start(a) if a == open || a == forget);
        draw_row_bg(app, frame, row, open);

        let folder = app.state.recent[index].clone();
        let name = app.recent_title(&folder);

        let cy = icon_center_y(app, row, app.fonts.ui);
        if app.recent_group(&folder).is_some() {
            icons::folders(frame, (row.x + 11.0, cy - 6.0), p.folder, editor);
        } else {
            icons::folder(frame, (row.x + 11.0, cy - 6.0), false, p.folder, editor);
        }

        // ✕ 只在悬停时显示，但其位置始终预留，名称和路径不会因 ✕ 出现而移动。
        let close = Rect::new(
            row.right() - 4.0 - BUTTON_SMALL,
            row.y + (ROW - BUTTON_SMALL) / 2.0,
            BUTTON_SMALL,
            BUTTON_SMALL,
        );
        // 项目文件名称后加淡色标注，样式同侧栏的「未保存」。
        let tag = app.recent_tag(&folder);
        let tag_w = tag.map_or(0.0, |tag| {
            6.0 + frame.measure(FontKind::Ui, app.fonts.ui, tag)
        });
        let room = (close.x - 8.0 - tag_w - (row.x + 36.0)).max(0.0);
        let name = ellipsize(frame, &name, app.fonts.ui, room);
        let mut end = ui_text(frame, app, row.x + 36.0, row, &name, p.text, editor);
        if let Some(tag) = tag {
            end = ui_text(frame, app, end + 6.0, row, tag, p.text_muted, editor);
        }

        // 名称之后是淡色的上级目录（用户主目录缩写为 `~`），放不下时截短，同名目录保留区分
        // 彼此的部分（见 `fit_parent`）。
        let room = close.x - 8.0 - (end + 10.0);
        if let Some(parent) = parents.get(index).filter(|p| !p.text.is_empty())
            && room > 40.0
        {
            let parent = fit_parent(frame, parent, app.fonts.ui_small, room);
            ui_text_small(frame, app, end + 10.0, row, &parent, p.text_dim, editor);
        }

        if hovered {
            draw_button(
                frame,
                p,
                close,
                editor,
                app.pointer.hover == Hover::Start(forget),
                false,
            );
            icons::close(frame, center(close), 6.5, p.text_muted, editor);
        }

        // ✕ 的命中区域排在整行之前：二者都包含鼠标时，先匹配到的是 ✕。
        app.hits.start.push((close, forget));
        app.hits.start.push((row, open));
        y += ROW;
    }
}

/// 返回启动动画进度为 `intro` 时第 `index` 项内容的不透明度和纵向位移（逻辑像素）：淡入途中
/// 自下而上移到原位。没有在播放启动动画（`None`）时是最终的样子。
fn staging(intro: Option<f32>, index: usize) -> (f32, f32) {
    let alpha = intro.map_or(1.0, |t| crate::app::setup::content_opacity(t, index));
    (alpha, (1.0 - alpha) * SETUP_RISE)
}

/// 绘制「首次设置」的第一页或快捷键一页（`keys`），整体下移 `rise`（逻辑像素）。
fn draw_setup_page(app: &mut App, frame: &mut Frame<'_>, keys: bool, rise: f32) {
    if keys {
        draw_setup_keys(app, frame, rise);
    } else {
        draw_setup(app, frame, rise);
    }
}

/// 绘制只有「首次设置」的开始页，整体下移 `rise`，并登记各控件的点击区域。
///
/// 自上而下是应用名称、一句说明、六行设置（外观、主题、语言和三个模块）和右对齐的「完成」按钮。
/// 每行左边是图标和名称，右边是一个控件，各控件的右端对齐：外观是分段控件，
/// 主题和语言是点击后弹出菜单的值，三个模块各是一个开关。启动动画期间先是应用图标在编辑区中央逐层出
/// 现、停留、淡出，各项随后逐个淡入；内容开始淡入之前各控件不响应点击。
fn draw_setup(app: &mut App, frame: &mut Frame<'_>, rise: f32) {
    use crate::keymap::Module;
    use winc_config::ThemeChoice;

    let p = app.palette;
    let clip = app.layout.editor;
    // 标题块的高度（逻辑像素）：应用名 34、说明 20，再留 2 的间隔。
    let title_h = 56.0;
    let rows = SETUP_ROWS * SETUP_ROW;
    let total = title_h + GAP + rows + GAP + SETUP_BUTTON_H;
    let w = WIDTH.min(clip.w - 48.0).max(0.0);
    let x = (clip.x + (clip.w - w) / 2.0).round();
    let top = (clip.y + ((clip.h - total) / 2.0 - 24.0).max(16.0) + rise).round();
    let intro = app.motion.setup_intro.map(|clock| clock.linear());

    if let Some(splash) = intro.and_then(crate::app::setup::splash) {
        let size = (SPLASH_LOGO * frame.scale()).round() as u32;
        // 图标完整之后（停留、淡出）每帧都是同一张图：用留存的应用图标，不重新渲染。
        let built;
        let pixels: &[u8] = if splash.build < 1.0 {
            built = crate::app::setup::logo_frame(size, splash.build);
            &built
        } else {
            app.product_logo(size)
        };
        let at = (
            clip.x + (clip.w - SPLASH_LOGO) / 2.0,
            clip.y + (clip.h - SPLASH_LOGO) / 2.0 - 24.0 - splash.rise,
        );
        frame.with_opacity(splash.opacity, |frame| {
            frame.pixels(at, size, size, pixels, clip);
        });
    }

    let mut y = top;
    let (alpha, rise) = staging(intro, 0);
    let title = Rect::new(x + 10.0, y + rise, w, 34.0);
    frame.with_opacity(alpha, |frame| {
        let size = app.fonts.ui + 13.0;
        let name = crate::layout::PRODUCT_NAME;
        crate::view::label(frame, app, size, (title.x, title), name, p.text, clip);
    });
    let (alpha, rise) = staging(intro, 1);
    let about = Rect::new(x + 10.0, y + 34.0 + rise, w, 20.0);
    frame.with_opacity(alpha, |frame| {
        let text = t!("workspace-start-setup-intro");
        ui_text_small(frame, app, about.x, about, text, p.text_dim, clip);
    });
    y += title_h + GAP;

    // 第 `index` 行（内容的第 `index + 2` 项）的区域、不透明度和图标的中心。
    let line = |app: &App, index: usize| {
        let (alpha, rise) = staging(intro, index + 2);
        let row = Rect::new(x, y + index as f32 * SETUP_ROW + rise, w, SETUP_ROW);
        let icon = (row.x + 18.0, icon_center_y(app, row, app.fonts.ui));
        (row, alpha, icon)
    };
    let named = |app: &App, frame: &mut Frame<'_>, row: Rect, name: &str| {
        ui_text(frame, app, row.x + 36.0, row, name, p.text, clip);
    };

    let (row, alpha, icon) = line(app, 0);
    frame.with_opacity(alpha, |frame| {
        named(app, frame, row, t!("settings-page-appearance"));
        icons::appearance(frame, icon, p.text_muted, clip);
        let choices = [
            (t!("settings-theme-auto"), ThemeChoice::Auto),
            (t!("settings-theme-light"), ThemeChoice::Light),
            (t!("settings-theme-dark"), ThemeChoice::Dark),
        ];
        // 换了外观时，凸起的那一段从原来的位置滑过来。
        let slide = app.settings.mode_slide.and_then(|(from, clock)| {
            let from = choices.iter().position(|(_, choice)| *choice == from)?;
            Some((from, clock))
        });
        let options = choices.map(|(label, choice)| {
            let action = StartAction::Appearance(choice);
            (label, choice == app.theme_choice, action)
        });
        draw_segments(app, frame, row, &options, slide);
    });

    let (row, alpha, icon) = line(app, 1);
    frame.with_opacity(alpha, |frame| {
        named(app, frame, row, t!("workspace-start-theme"));
        icons::sidebar(frame, icon, true, p.text_muted, clip);
        let theme = crate::theme::themes(p.dark)[app.theme_slot(p.dark)].name();
        draw_dropdown(app, frame, row, theme, StartAction::Theme);
    });

    let (row, alpha, icon) = line(app, 2);
    frame.with_opacity(alpha, |frame| {
        named(app, frame, row, t!("settings-language"));
        icons::globe(frame, icon, 1.0, p.text_muted, clip);
        let language = crate::layout::locale_label(app.config.language.unwrap_or_default());
        draw_dropdown(app, frame, row, language, StartAction::Language);
    });

    let (row, alpha, icon) = line(app, 3);
    frame.with_opacity(alpha, |frame| {
        named(app, frame, row, "Git");
        icons::git(frame, icon, 1.0, p.text_muted, clip);
        draw_switch(app, frame, row, app.git_enabled(), Module::Git);
    });

    let (row, alpha, icon) = line(app, 4);
    frame.with_opacity(alpha, |frame| {
        named(app, frame, row, t!("settings-terminal-module"));
        icons::terminal(frame, icon, p.text_muted, clip);
        draw_switch(app, frame, row, app.terminal_enabled(), Module::Terminal);
    });

    let (row, alpha, icon) = line(app, 5);
    frame.with_opacity(alpha, |frame| {
        named(app, frame, row, t!("workspace-start-module-remote"));
        icons::server(frame, icon, p.text_muted, clip);
        draw_switch(app, frame, row, app.remote_enabled(), Module::Remote);
    });

    let (alpha, rise) = staging(intro, 8);
    let label = t!("workspace-start-setup-next");
    let width = (frame.measure(FontKind::Ui, app.fonts.ui, label) + 32.0).round();
    let top = y + rows + GAP + rise;
    let button = Rect::new(x + w - 6.0 - width, top, width, SETUP_BUTTON_H);
    let action = StartAction::SetupNext;
    let hovered = app.pointer.hover == Hover::Start(action);
    frame.with_opacity(alpha, |frame| {
        draw_action_button(
            app,
            frame,
            button,
            label,
            ButtonRole::Primary,
            true,
            hovered,
        );
    });
    app.hits.start.push((button, action));
    // 内容开始淡入之前页面上只有图标：看不见的控件不响应点击。
    if intro.is_some_and(|t| t < crate::app::setup::CONTENT_START) {
        app.hits.start.clear();
    }
}

/// 绘制「首次设置」的快捷键一页：标题、一句说明、各键位方案、小标题下从本机找到的编辑器，以及
/// 右对齐的「返回」「完成」按钮，整体下移 `rise`。选中的一项右端有对勾，默认方案的名称后注明
/// 「默认」。
fn draw_setup_keys(app: &mut App, frame: &mut Frame<'_>, rise: f32) {
    use crate::keymap::Preset;

    let p = app.palette;
    let clip = app.layout.editor;
    let sources: Vec<String> = (app.setup.sources.iter()).map(|s| s.name.clone()).collect();
    let title_h = 56.0;
    let heading_h = if sources.is_empty() { 0.0 } else { 32.0 };
    let count = Preset::ALL.len() + sources.len();
    let rows = count as f32 * SETUP_ROW + heading_h;
    let total = title_h + GAP + rows + GAP + SETUP_BUTTON_H;
    let w = WIDTH.min(clip.w - 48.0).max(0.0);
    let x = (clip.x + (clip.w - w) / 2.0).round();
    let mut y = (clip.y + ((clip.h - total) / 2.0 - 24.0).max(16.0) + rise).round();

    let title = Rect::new(x + 10.0, y, w, 34.0);
    let size = app.fonts.ui + 13.0;
    let heading = t!("workspace-start-keymap-title");
    crate::view::label(frame, app, size, (title.x, title), heading, p.text, clip);
    let about = Rect::new(x + 10.0, y + 34.0, w, 20.0);
    let text = t!("workspace-start-keymap-intro");
    ui_text_small(frame, app, about.x, about, text, p.text_dim, clip);
    y += title_h + GAP;
    let list = y;

    // 先是各方案，其后在小标题下列出本机找到的编辑器。
    for i in 0..count {
        let imported = i >= Preset::ALL.len();
        if i == Preset::ALL.len() {
            let line = Rect::new(x + 10.0, y + 8.0, w, heading_h - 8.0);
            let text = t!("workspace-start-keymap-import");
            ui_text_small(frame, app, line.x, line, text, p.text_dim, clip);
            y += heading_h;
        }
        let row = Rect::new(x, y, w, SETUP_ROW);
        y += SETUP_ROW;
        let action = StartAction::SetupKeymap(i);
        draw_row_bg(app, frame, row, action);
        let cy = icon_center_y(app, row, app.fonts.ui);
        if imported {
            icons::download(frame, (row.x + 18.0, cy), p.text_muted, clip);
        } else {
            icons::keyboard(frame, (row.x + 18.0, cy), p.text_muted, clip);
        }
        let name = match Preset::ALL.get(i) {
            Some(preset) => preset.title(),
            None => sources[i - Preset::ALL.len()].as_str(),
        };
        let end = ui_text(frame, app, row.x + 36.0, row, name, p.text, clip);
        if Preset::ALL.get(i) == Some(&Preset::default()) {
            let note = t!("workspace-start-keymap-default");
            ui_text_small(frame, app, end + 8.0, row, note, p.text_dim, clip);
        }
        if app.setup.choice == i {
            icons::check(frame, (row.right() - 18.0, cy), 10.0, p.accent, clip);
        }
        app.hits.start.push((row, action));
    }
    let y = list;

    let top = y + rows + GAP;
    let mut right = x + w - 6.0;
    for (label, role, action) in [
        (
            t!("workspace-start-setup-done"),
            ButtonRole::Primary,
            StartAction::FinishSetup,
        ),
        (
            t!("workspace-start-setup-back"),
            ButtonRole::Normal,
            StartAction::SetupBack,
        ),
    ] {
        let width = (frame.measure(FontKind::Ui, app.fonts.ui, label) + 32.0).round();
        let button = Rect::new(right - width, top, width, SETUP_BUTTON_H);
        let hovered = app.pointer.hover == Hover::Start(action);
        draw_action_button(app, frame, button, label, role, true, hovered);
        app.hits.start.push((button, action));
        right = button.x - 8.0;
    }
}

/// 在 `row` 的右端绘制点击后弹出菜单的值并登记点击区域，样式与设置中弹出菜单的值相同：右
/// 对齐的文字后跟一对上下小箭头，悬停时铺悬停底色。
///
/// 箭头的右端与其他行的控件对齐，悬停底色因此比其他控件向右多出一段。
fn draw_dropdown(app: &mut App, frame: &mut Frame<'_>, row: Rect, text: &str, action: StartAction) {
    use crate::theme::SETTINGS_GLYPH_PAD;
    let clip = app.layout.editor;
    let text_w = frame.measure(FontKind::Ui, app.fonts.ui, text);
    // 文字左边留 10，文字之后的 16 是箭头和间隔，再加右边的边距。
    let width = (10.0 + text_w + 16.0 + SETTINGS_GLYPH_PAD).round();
    let control = Rect::new(
        row.right() - 6.0 + SETTINGS_GLYPH_PAD - 1.0 - width,
        (row.y + (row.h - DROPDOWN_H) / 2.0).round(),
        width,
        DROPDOWN_H,
    );
    let hovered = app.pointer.hover == Hover::Start(action);
    crate::view::controls::popup_value(app, frame, control, text, false, hovered, clip);
    app.hits.start.push((control, action));
}

/// 在 `row` 的右端绘制一个分段控件并登记各段的点击区域：一条底槽，选中的那一段凸起，悬停的
/// 那一段铺浅色，样式与设置中「模式」的分段控件相同。各段等宽，按最宽的文字定。
///
/// `slide` 是选中项刚换过时原来那一段的序号和过渡的时钟：凸起的那一段从原来的位置滑过来。
fn draw_segments(
    app: &mut App,
    frame: &mut Frame<'_>,
    row: Rect,
    options: &[(&str, bool, StartAction)],
    slide: Option<(usize, crate::app::animation::Clock)>,
) {
    let p = app.palette;
    let clip = app.layout.editor;
    let size = app.fonts.ui_small;
    let widest = (options.iter())
        .map(|(label, ..)| frame.measure(FontKind::Ui, size, label))
        .fold(0.0, f32::max);
    let segment_w = (widest + SEGMENT_PAD * 2.0).round();
    let width = segment_w * options.len() as f32 + 4.0;
    let track = Rect::new(
        row.right() - 6.0 - width,
        (row.y + (row.h - SEGMENTS_H) / 2.0).round(),
        width,
        SEGMENTS_H,
    );
    let segment = |index: usize| {
        Rect::new(
            track.x + 2.0 + index as f32 * segment_w,
            track.y + 2.0,
            segment_w,
            track.h - 4.0,
        )
    };
    let selected = options.iter().position(|(_, selected, _)| *selected);
    let slot = selected.map(segment).map(|to| match slide {
        Some((from, clock)) => Rect::new(clock.between(segment(from).x, to.x), to.y, to.w, to.h),
        None => to,
    });
    crate::view::controls::segment_track(frame, p, track, p.editor_bg, slot, clip);
    for (index, &(label, selected, action)) in options.iter().enumerate() {
        let segment = segment(index);
        let hovered = app.pointer.hover == Hover::Start(action);
        if hovered && !selected {
            frame.rect_clipped(segment, p.hover, RADIUS_ROW, clip);
        }
        let color = if selected || hovered {
            p.text
        } else {
            p.text_muted
        };
        let text_w = frame.measure(FontKind::Ui, size, label);
        let at = (segment.x + (segment.w - text_w) / 2.0).round();
        ui_text_small(frame, app, at, segment, label, color, clip);
        app.hits.start.push((segment, action));
    }
}

/// 在 `row` 的右端绘制模块 `module` 的开关并登记点击区域，样式与设置中的开关相同；刚切换时
/// 圆钮滑到另一侧，强调色随之淡入或淡出。
fn draw_switch(
    app: &mut App,
    frame: &mut Frame<'_>,
    row: Rect,
    on: bool,
    module: crate::keymap::Module,
) {
    use crate::keymap::Module;
    use crate::layout::Setting;
    use crate::theme::{SETTINGS_SWITCH_H, SETTINGS_SWITCH_W};
    let clip = app.layout.editor;
    let track = Rect::new(
        row.right() - 6.0 - SETTINGS_SWITCH_W,
        (row.y + (row.h - SETTINGS_SWITCH_H) / 2.0).round(),
        SETTINGS_SWITCH_W,
        SETTINGS_SWITCH_H,
    );
    let setting = match module {
        Module::Git => Setting::GitModule,
        Module::Terminal => Setting::TerminalModule,
        Module::Remote => Setting::RemoteModule,
    };
    // 开启程度：0 为关闭，1 为开启，切换途中从原状态过渡到新状态。
    let end = if on { 1.0 } else { 0.0 };
    let t = match app.settings.switch_slide {
        Some((sliding, clock)) if sliding == setting => clock.between(1.0 - end, end),
        _ => end,
    };
    crate::view::controls::switch(frame, app.palette, track, t, clip);
    // 点击区域比开关大一圈，便于点中。
    app.hits
        .start
        .push((track.inset(-6.0, -6.0), StartAction::Module(module)));
}

/// 绘制一段的小标题，返回其下方内容起始位置的 y 坐标。
fn section_title(app: &App, frame: &mut Frame<'_>, text: &str, x: f32, y: f32, w: f32) -> f32 {
    let rect = Rect::new(x + 10.0, y, w, SECTION_TITLE);
    let p = app.palette;
    ui_text_small(
        frame,
        app,
        rect.x,
        rect,
        text,
        p.text_dim,
        app.layout.editor,
    );
    y + SECTION_TITLE
}

/// 绘制悬停时的圆角底色，样式与文件树的行相同。
fn draw_row_bg(app: &App, frame: &mut Frame<'_>, row: Rect, action: StartAction) {
    let hovered = match app.pointer.hover {
        Hover::Start(StartAction::Forget(i)) => action == StartAction::Recent(i),
        Hover::Start(a) => a == action,
        _ => false,
    };
    if hovered {
        frame.rounded(row, app.palette.hover, RADIUS_ROW);
    }
}

/// 绘制操作行前面的图标，与欢迎页上同一操作的图标相同。
fn action_icon(app: &App, frame: &mut Frame<'_>, action: StartAction, row: Rect) {
    let p = app.palette;
    let (cx, cy) = (row.x + 18.0, icon_center_y(app, row, app.fonts.ui));
    let clip = app.layout.editor;
    let color = p.text_muted;

    // 图标与空编辑区页上同一操作的图标相同。
    match action {
        StartAction::Settings => icons::gear(frame, (cx, cy), color, clip),
        StartAction::NewFile => icons::new_file(frame, (cx - 6.0, cy - 7.5), color, clip),
        StartAction::Open if !cfg!(target_os = "macos") => {
            icons::files(frame, (cx, cy), 1.0, color, clip);
        }
        StartAction::Open | StartAction::OpenFolder => {
            icons::folder(frame, (cx - 7.5, cy - 6.0), true, color, clip);
        }
        StartAction::Remote => icons::server(frame, (cx, cy), color, clip),
        StartAction::Recent(_)
        | StartAction::Forget(_)
        | StartAction::Language
        | StartAction::Appearance(_)
        | StartAction::Theme
        | StartAction::Module(_)
        | StartAction::SetupNext
        | StartAction::SetupBack
        | StartAction::SetupKeymap(_)
        | StartAction::FinishSetup => {}
    }
}
