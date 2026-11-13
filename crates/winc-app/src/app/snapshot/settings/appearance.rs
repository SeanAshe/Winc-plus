//! 外观设置的测试：字体、主题、配色和背景图。

use super::*;

#[test]
fn font_size_keys_change_the_editor_font_size_and_reset() {
    let dir = search_folder("zoom");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let ui = app.fonts.ui;

    app.step_font_size(Some(true));
    assert_eq!(app.config.editor_size, Some(CODE_FONT + 1.0));
    assert_eq!(app.fonts.code, CODE_FONT + 1.0);
    // 界面字号保持不变。
    assert_eq!(app.fonts.ui, ui);

    app.step_font_size(Some(false));
    app.step_font_size(Some(false));
    assert_eq!(app.fonts.code, CODE_FONT - 1.0);

    // ⌘0 还原：配置中不再记录字号。
    app.step_font_size(None);
    assert_eq!(app.config.editor_size, None);
    assert_eq!(app.fonts.code, CODE_FONT);

    // 已达上限时再放大：字号不变，并在状态栏提示。
    app.config.editor_size = Some(*Config::EDITOR_SIZE_RANGE.end());
    app.step_font_size(Some(true));
    assert_eq!(
        app.config.editor_size,
        Some(*Config::EDITOR_SIZE_RANGE.end())
    );
    assert!(
        app.status
            .message
            .as_deref()
            .is_some_and(|m| m.contains("无法继续调整"))
    );
    // 测试中没有配置文件路径，不会写入用户的配置。
    assert!(app.store.config_path.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_font_name_is_checked_and_can_be_abandoned() {
    let dir = search_folder("settings-font");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_start_editing(Setting::EditorFont);
    assert_eq!(app.focus, Focus::Setting);

    app.edit_field(|f| {
        f.select_all();
        f.insert("绝对不存在的字体 ZZZ")
    });
    app.settings_commit_field();

    // 字体找不到：按 ↩ 提交时停留在输入框并显示错误，配置退回原值。
    assert!(app.settings.editing.is_some());
    assert_eq!(
        app.settings.error,
        Some((
            Setting::EditorFont,
            "未安装字体「绝对不存在的字体 ZZZ」".to_owned()
        ))
    );

    // 找得到但画不出字形的字体（例如字形只放在 `hvgl` 表中）同样不能提交：渲染层会静默改用
    // 默认字体，界面字体设成这种字体时所有文字都是空白。
    let font = dir.join("hvgl.ttf");
    let mut data = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    data.extend(b"hvgl");
    data.extend([0, 0, 0, 0, 0, 0, 0, 28, 0, 0, 0, 0]);
    std::fs::write(&font, data).unwrap();
    let path = font.to_string_lossy().into_owned();
    app.edit_field(|f| {
        f.select_all();
        f.insert(&path)
    });
    app.settings_commit_field();
    assert!(app.settings.editing.is_some());
    assert_eq!(
        app.settings.error,
        Some((
            Setting::EditorFont,
            format!("不支持字体「{path}」的字形格式")
        ))
    );

    // 配置中原本就是用不了的字体（例如手动改了配置文件）：打开设置、换到该页时在该行写明原因。
    app.settings_cancel_field();
    app.close_settings();
    app.config.editor_font = Some(path.clone());
    app.open_settings();
    assert_eq!(app.settings.error, None, "第一页上没有编辑器字体");
    app.settings_show_page(SettingsPage::Editor);
    assert!(matches!(
        &app.settings.error,
        Some((Setting::EditorFont, _))
    ));
    app.close_settings();
    app.config.editor_font = None;
    app.open_settings();
    app.settings_start_editing(Setting::EditorFont);
    app.edit_field(|f| {
        f.select_all();
        f.insert("绝对不存在的字体 ZZZ")
    });

    // Esc 只放弃本次输入，浮层保持打开，焦点回到浮层。
    app.settings_cancel_field();
    assert!(app.settings.open);
    assert!(app.settings.editing.is_none());
    assert_eq!(app.focus, Focus::Settings);
    assert_eq!(app.config.editor_font, None);

    // 输入到一半就关闭浮层：不完整的名称不保留在配置中。
    app.settings_start_editing(Setting::EditorFont);
    app.edit_field(|f| {
        f.select_all();
        f.insert("JetB")
    });
    assert_eq!(app.config.editor_font.as_deref(), Some("JetB"));
    app.close_settings();
    assert_eq!(app.config.editor_font, None);
    assert_eq!(app.focus, Focus::Editor);

    // 再次打开：从第一页的顶部开始。
    app.open_settings();
    assert_eq!(app.settings.selected, Setting::Theme);
    assert_eq!(app.settings_layout().page, SettingsPage::Appearance);
    assert_eq!(app.settings.scroll, 0.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_font_can_be_chosen_from_the_menu() {
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};
    use crate::layout::{SettingsPart, font_menu_button};

    let dir = search_folder("settings-font-menu");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_show_page(Setting::EditorFont.page());
    paint(&mut app);

    // 点输入框右端的按钮弹出字体菜单；点其余位置仍是输入框。
    let row = *app.settings_layout().row(Setting::EditorFont).unwrap();
    let button = font_menu_button(row.control);
    let (x, y) = (button.x + button.w / 2.0, button.y + button.h / 2.0);
    assert_eq!(
        app.settings_part_at(x, y),
        Some(SettingsPart::FontMenu(Setting::EditorFont))
    );
    assert_eq!(
        app.settings_part_at(button.x - 4.0, y),
        Some(SettingsPart::Field(Setting::EditorFont))
    );
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(
        app.overlays.context_menu.as_ref().map(|m| &m.target),
        Some(&MenuTarget::Font(Setting::EditorFont))
    );
    assert!(app.settings.editing.is_none());
    assert!(app.menu_checked(MenuCommand::FontDefault));
    app.overlays.context_menu = None;
    paint(&mut app);

    // 编辑器和终端字体把等宽字体排在前面，其余字体仍在同一级菜单里；界面字体按名字列出。
    let fonts = winc_render::installed_fonts();
    let listed = |setting| {
        crate::app::input::context_menu::font_menu(setting)
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    crate::app::input::context_menu::MenuEntry::Item(MenuCommand::Font(_), _)
                )
            })
            .count()
    };
    assert_eq!(listed(Setting::UiFont), fonts.len());
    assert_eq!(listed(Setting::EditorFont), fonts.len());

    #[cfg(target_os = "macos")]
    {
        // 选中后立刻生效并写入配置，再选「系统默认」清除。
        let menlo = fonts.iter().position(|f| f.name == "Menlo").unwrap();
        app.settings_open_menu(Setting::EditorFont);
        app.run_menu_command(MenuCommand::Font(menlo));
        assert_eq!(app.config.editor_font.as_deref(), Some("Menlo"));
        assert_eq!(app.settings.error, None);

        app.settings_open_menu(Setting::EditorFont);
        assert!(app.menu_checked(MenuCommand::Font(menlo)));
        app.run_menu_command(MenuCommand::FontDefault);
        assert_eq!(app.config.editor_font, None);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_language_and_ui_font_controls_line_up() {
    let dir = search_folder("settings-language-width");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_show_page(Setting::Language.page());
    let layout = app.settings_layout();
    let control = |setting| layout.row(setting).unwrap().control;
    let (language, font) = (control(Setting::Language), control(Setting::UiFont));
    assert_eq!((language.x, language.w), (font.x, font.w));
    assert_eq!(Setting::Language.description(), None);
    // 与模式分段控件同宽、右缘对齐；菜单的值的底色向右多出内边距。
    let theme = control(Setting::Theme);
    let pad = crate::theme::SETTINGS_GLYPH_PAD;
    assert_eq!((language.x - pad, language.w), (theme.x, theme.w));

    // 语言菜单比控件短，打开后与控件同宽并对齐。
    app.settings_open_menu(Setting::Language);
    paint(&mut app);
    let popup = app
        .overlays
        .context_menu
        .as_ref()
        .unwrap()
        .popup
        .as_ref()
        .unwrap();
    let menu = popup.levels[0].rect;
    assert_eq!((menu.x, menu.w), (language.x, language.w));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_font_weight_steps_by_hundreds_within_its_range() {
    let dir = sample_folder("settings-font-weight");
    let mut app = app_with(&dir.join("short.rs"), false);
    assert_eq!(Setting::FontWeight.page(), SettingsPage::Appearance);
    assert_eq!(app.setting_value(Setting::FontWeight).0, "400");

    app.settings_adjust(Setting::FontWeight, true);
    assert_eq!(app.config.ui_font_weight, Some(500));
    // 回到 400 时从配置中去掉该项。
    app.settings_adjust(Setting::FontWeight, false);
    assert_eq!(app.config.ui_font_weight, None);

    // 不在档位上的值移到该方向上最近的一档。
    app.config.ui_font_weight = Some(450);
    app.settings_adjust(Setting::FontWeight, true);
    assert_eq!(app.config.ui_font_weight, Some(500));
    app.config.ui_font_weight = Some(450);
    app.settings_adjust(Setting::FontWeight, false);
    assert_eq!(app.config.ui_font_weight, None);

    // 到上下限后不再变化。
    app.config.ui_font_weight = Some(900);
    app.settings_adjust(Setting::FontWeight, true);
    assert_eq!(app.config.ui_font_weight, Some(900));
    assert_eq!(
        app.setting_value(Setting::FontWeight),
        ("900".to_owned(), false, true)
    );
    app.config.ui_font_weight = Some(100);
    app.settings_adjust(Setting::FontWeight, false);
    assert_eq!(app.config.ui_font_weight, Some(100));
    assert_eq!(
        app.setting_value(Setting::FontWeight),
        ("100".to_owned(), true, false)
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 各档字重的截图：`cargo test -p winc-app shoot_font_weights -- --ignored`。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_font_weights() {
    let dir = sample_folder("settings-font-weights");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.run(Command::Insert("// 字重 weight 0123 fn main() {}\n".into()));
    for weight in [300, 400, 500, 600, 700] {
        app.config.ui_font_weight = Some(weight);
        app.apply_fonts().unwrap();
        shoot(&mut app, &format!("font-weight-{weight}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 背景图片：配置后在后台生成铺满窗口（按逻辑像素）的图层，底色之下透出图片；不透明度、配色
/// 改变后随之更新，取消后恢复纯色；无法加载的图片提示一次，不反复重试。生成好的图层写入磁盘
/// 缓存，之后直接读取，不再解码原图。
#[test]
fn the_background_image_shows_under_the_base_colors() {
    let dir = sample_folder("settings-background");
    // 一张纯红的图片，比窗口大。
    let image = dir.join("红.png");
    let red = winc_render::Image {
        pixels: vec![0xffff_0000; 2000 * 1500],
        width: 2000,
        height: 1500,
    };
    write_png(&image, &red).unwrap();
    let mut app = app_with(&dir.join("short.rs"), false);
    // 状态文件所在的目录即缓存所在的目录。
    app.store.state_path = Some(dir.join("state.json"));
    let cache = dir.join("wallpaper.cache");
    assert_eq!(Setting::BackgroundImage.page(), SettingsPage::Appearance);
    assert_eq!(app.setting_value(Setting::BackgroundOpacity).0, "15");

    let pixel = |app: &mut App, (x, y): (f32, f32)| {
        paint(app);
        app.wait_for_wallpaper();
        paint(app);
        let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
        let at = (y * SCALE) as usize * image.width as usize + (x * SCALE) as usize;
        let px = image.pixels[at];
        ((px >> 16) & 0xff, (px >> 8) & 0xff, px & 0xff)
    };
    // 编辑区右下角空白处的一个像素（只有底色）。
    let sample = |app: &mut App| {
        let editor = app.layout.editor;
        pixel(app, (editor.right() - 40.0, editor.bottom() - 40.0))
    };
    // 设置卡片左下角的一个像素（只有卡片的底色）。
    let card = |app: &mut App| {
        app.open_settings();
        let card = app.settings_layout().card;
        let px = pixel(app, (card.x + 10.0, card.bottom() - 10.0));
        app.close_settings();
        px
    };
    let plain = sample(&mut app);
    let plain_card = card(&mut app);
    let mixed = |base: (u32, u32, u32), opacity: f32| {
        let part = |c: u32| (c as f32 * (1.0 - opacity)).round() as u32;
        let red = (255 * (opacity * 256.0) as u32) >> 8;
        (part(base.0) + red, part(base.1), part(base.2))
    };

    app.set_background_image(Some(image.clone()));
    assert_eq!(sample(&mut app), mixed(plain, 0.15));
    // 图层保持图片的宽高比，缩小到刚好铺满窗口（按逻辑像素，不是物理像素）。
    let size = app.frame.renderer.as_ref().unwrap().wallpaper_size();
    assert_eq!((SIZE.0, SIZE.1), (1280.0, 820.0));
    assert_eq!(size, Some((1280, 960)));
    // 浮在内容之上的设置卡片不透出背景图。
    assert_eq!(card(&mut app), plain_card);

    // 图层已写入缓存。把缓存中的像素改成绿色后调整不透明度：读的是缓存，不再解码原图。
    let mut bytes = std::fs::read(&cache).unwrap();
    let pixels = bytes.len() - 1280 * 960 * 4;
    for px in bytes[pixels..].as_chunks_mut::<4>().0 {
        *px = [0, 255, 0, 255];
    }
    std::fs::write(&cache, &bytes).unwrap();
    app.settings_adjust(Setting::BackgroundOpacity, true);
    let green = sample(&mut app);
    assert!(green.1 > green.0 && green.1 > green.2, "{green:?}");
    // 换回原来的像素，后面的检查不受影响。
    for px in bytes[pixels..].as_chunks_mut::<4>().0 {
        *px = [255, 0, 0, 255];
    }
    std::fs::write(&cache, &bytes).unwrap();
    app.settings_adjust(Setting::BackgroundOpacity, false);
    assert_eq!(sample(&mut app), mixed(plain, 0.15));

    // 调高不透明度：重新生成。
    app.settings_adjust(Setting::BackgroundOpacity, true);
    assert_eq!(app.config.ui_background_opacity, Some(20.0));
    assert_eq!(sample(&mut app), mixed(plain, 0.2));

    // 范围是 0 到 100：调到 0 时按没有背景图处理，图层释放；调回来后重新生成。
    app.config.ui_background_opacity = Some(5.0);
    app.settings_adjust(Setting::BackgroundOpacity, false);
    app.settings_adjust(Setting::BackgroundOpacity, false);
    assert_eq!(app.config.ui_background_opacity, Some(0.0));
    assert_eq!(sample(&mut app), plain);
    assert_eq!(app.frame.renderer.as_ref().unwrap().wallpaper_size(), None);
    app.config.ui_background_opacity = Some(95.0);
    app.settings_adjust(Setting::BackgroundOpacity, true);
    app.settings_adjust(Setting::BackgroundOpacity, true);
    assert_eq!(app.config.ui_background_opacity, Some(100.0));
    assert_eq!(sample(&mut app), (255, 0, 0));
    // 不透明度超过一半：图层按物理像素生成，这里即图片原来的大小。
    let size = |app: &App| app.frame.renderer.as_ref().unwrap().wallpaper_size();
    assert_eq!(size(&app), Some((2000, 1500)));
    app.config.ui_background_opacity = Some(20.0);
    assert_eq!(sample(&mut app), mixed(plain, 0.2));

    // 显示方式。适应：图片按高度缩小并居中，取样处在图片之外，只有底色。
    use winc_config::BackgroundFit;
    assert_eq!(Setting::BackgroundFit.page(), SettingsPage::Appearance);
    app.set_background_fit(BackgroundFit::Fit);
    assert_eq!(sample(&mut app), plain);
    assert_eq!(size(&app), Some((1093, 820)));
    let middle = (SIZE.0 / 2.0, app.layout.editor.bottom() - 40.0);
    assert_eq!(pixel(&mut app, middle), mixed(plain, 0.2));
    // 拉伸：铺满窗口。
    app.set_background_fit(BackgroundFit::Stretch);
    assert_eq!(sample(&mut app), mixed(plain, 0.2));
    // 平铺：图层保持图片的大小（不超过上限）。
    app.set_background_fit(BackgroundFit::Tile);
    assert_eq!(sample(&mut app), mixed(plain, 0.2));
    assert_eq!(size(&app), Some((1920, 1440)));
    app.set_background_fit(BackgroundFit::Fill);
    assert_eq!(sample(&mut app), mixed(plain, 0.2));

    // 换成深色配色：底色随之更换，图片不必重新生成。
    app.set_theme(winit::window::Theme::Dark);
    let dark = {
        let c = app.palette.editor_bg;
        let part = |v: f32| (v * 255.0 + 0.5) as u32;
        (part(c.r), part(c.g), part(c.b))
    };
    paint(&mut app);
    assert!(!app.wallpaper.is_busy());
    assert_eq!(sample(&mut app), mixed(dark, 0.2));

    // 取消：恢复纯色，图层释放。
    app.set_background_image(None);
    assert_eq!(sample(&mut app), dark);
    assert_eq!(app.frame.renderer.as_ref().unwrap().wallpaper_size(), None);

    // 无法加载的图片：保持纯色，之后不再重试。
    app.set_background_image(Some(dir.join("不存在.png")));
    assert_eq!(sample(&mut app), dark);
    paint(&mut app);
    assert!(!app.wallpaper.is_busy());

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 截取带背景图的界面：图片取环境变量 `WINC_BACKGROUND`，每套主题各截主界面、文件面板、设置
/// 和各显示方式。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_background_image() {
    let Some(image) = std::env::var_os("WINC_BACKGROUND") else {
        return;
    };
    let dir = sample_folder("shoot-background");
    let themes = [
        (false, None, "light"),
        (true, None, "dark"),
        (false, Some("veil-snow"), "snow"),
        (true, Some("veil-night"), "night"),
    ];
    for (dark, id, name) in themes {
        for opacity in [30.0, 60.0, 100.0] {
            let mut app = app_with(&dir.join("src/main.rs"), dark);
            app.config.ui_background_image = Some(image.to_string_lossy().into_owned());
            app.config.ui_background_opacity = Some(opacity);
            if dark {
                app.config.dark_theme = id.map(str::to_owned);
            } else {
                app.config.light_theme = id.map(str::to_owned);
            }
            let theme = if dark { Theme::Dark } else { Theme::Light };
            app.set_theme(theme);
            paint(&mut app);
            app.wait_for_wallpaper();
            shoot(&mut app, &format!("background-{name}-{opacity}"));
            if opacity > 30.0 {
                continue;
            }
            app.open_picker();
            shoot(&mut app, &format!("background-{name}-picker"));
            app.close_picker();
            app.open_settings();
            shoot(&mut app, &format!("background-{name}-settings"));
            app.close_settings();
            for (i, fit) in winc_config::BackgroundFit::ALL.into_iter().enumerate() {
                app.set_background_fit(fit);
                paint(&mut app);
                app.wait_for_wallpaper();
                shoot(&mut app, &format!("background-{name}-fit-{i}"));
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 有背景图时滚动设置页：使用快照的帧与完整绘制的帧相同，卡片不随之透出或不透出背景图。
#[test]
fn scrolling_settings_over_a_background_image_looks_the_same() {
    let dir = sample_folder("settings-scroll-background");
    // 一张渐变的图片：位置有偏差时像素不同。
    let image = dir.join("图.png");
    let (w, h) = (640, 400);
    let pixels = (0..w * h).map(|i| 0xff00_0000 | ((i % w * 255 / w) << 16) | (i / w * 255 / h));
    let gradient = winc_render::Image {
        pixels: pixels.collect(),
        width: w,
        height: h,
    };
    write_png(&image, &gradient).unwrap();
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.ui_background_image = Some(image.to_string_lossy().into_owned());
    app.config.ui_background_opacity = Some(60.0);
    paint(&mut app);
    app.wait_for_wallpaper();
    app.open_settings();
    app.settings.motion = None;
    let shot = |app: &mut App| {
        paint(app);
        app.frame
            .renderer
            .as_ref()
            .unwrap()
            .image()
            .unwrap()
            .pixels
            .clone()
    };
    shot(&mut app);

    app.settings_scroll_by(-10.0);
    shot(&mut app);
    app.settings_scroll_by(-30.0);
    let reused = shot(&mut app);
    let (window, scale) = (app.layout.window, SCALE);
    assert!(app.settings_reuses_base(window, scale));
    let later = std::time::Instant::now() + std::time::Duration::from_secs(1);
    assert_eq!(app.settings_scroll_tick(later), None);
    assert!(!app.settings_reuses_base(window, scale));
    let full = shot(&mut app);
    let differ = (reused.iter().zip(&full)).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "使用快照的画面与完整绘制不同");

    // 卡片左下角只有卡片的底色，不透出背景图。
    let card = app.settings_layout().card;
    let at =
        |x: f32, y: f32| (y * SCALE) as usize * (SIZE.0 * SCALE) as usize + (x * SCALE) as usize;
    let corner = full[at(card.x + 10.0, card.bottom() - 10.0)];
    let part = |v: f32| (v * 255.0 + 0.5) as u32;
    let c = app.palette.frame_bg;
    let plain = (part(c.r) << 16) | (part(c.g) << 8) | part(c.b);
    assert_eq!(corner & 0xff_ffff, plain);

    std::fs::remove_dir_all(&dir).unwrap();
}
