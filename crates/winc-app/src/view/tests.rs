//! 文本截短的测试：`fit` 保留的字数与宽度上限，以及路径截短后保留文件名。

use winc_render::Renderer;

use super::*;

/// 返回鼠标所在元素的提示（提示所依附的区域与文字）；见 `workspace::panel::tooltip`。
pub(crate) fn tooltip(app: &App) -> Option<(Rect, String)> {
    workspace::panel::tooltip(app)
}

pub(crate) use super::settings::draw_cached as settings_draw_cached;
pub(crate) use crate::app::workspace::tab_names::distinct_folder;

#[test]
fn fit_keeps_as_many_characters_as_the_width_allows() {
    let mut renderer = Renderer::offscreen(10.0, 10.0, 2.0, Color::hex(0), None, None)
        .expect("测试机上有系统字体");
    renderer.draw(|frame| {
        let size = 13.0;
        let text = "a-very-long-file-name_中文.rs";
        let chars: Vec<char> = text.chars().collect();
        let full = frame.measure(FontKind::Ui, size, text);
        let ellipsis = frame.measure(FontKind::Ui, size, "…");
        for max in [
            0.0,
            ellipsis,
            full * 0.3,
            full * 0.55,
            full * 0.8,
            full - 0.5,
        ] {
            for middle in [false, true] {
                let out = fit(frame, text, size, max, middle).into_owned();
                let (head, tail) = out.split_once('…').expect("截短以后带「…」");
                // 保留的是原文的开头（从中间截时还有结尾），开头不比结尾少。
                assert!(text.starts_with(head) && text.ends_with(tail), "{out}");
                let (head, tail) = (head.chars().count(), tail.chars().count());
                assert!(if middle { head - tail <= 1 } else { tail == 0 }, "{out}");
                // 只剩「…」以外都放得进去，再多留一个字就放不下。
                let width = frame.measure(FontKind::Ui, size, &out);
                assert!(head == 0 || width <= max + 1e-3, "{out} {max}");
                let keep = head + tail + 1;
                if middle && keep >= chars.len() {
                    continue;
                }
                let (head, tail) = if middle {
                    (keep - keep / 2, keep / 2)
                } else {
                    (keep, 0)
                };
                let longer: String = chars[..head]
                    .iter()
                    .chain(['…'].iter())
                    .chain(&chars[chars.len() - tail..])
                    .collect();
                let width = frame.measure(FontKind::Ui, size, &longer);
                assert!(width > max - 1e-3, "{out} {longer} {max}");
            }
        }
        // 放得下时原样返回。
        assert!(matches!(
            fit(frame, text, size, full + 1.0, true),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            fit(frame, "", size, 0.0, false),
            Cow::Borrowed("")
        ));
    });
}

#[test]
fn shortened_paths_keep_the_file_name() {
    let mut renderer = Renderer::offscreen(10.0, 10.0, 2.0, Color::hex(0), None, None)
        .expect("测试机上有系统字体");
    renderer.draw(|frame| {
        let size = 13.0;
        let path = "/private/var/folders/xy/abcdefgh/T/sample/Cargo.toml";
        let full = frame.measure(FontKind::Ui, size, path);
        assert!(matches!(
            shorten_path(frame, path, size, full),
            Cow::Borrowed(_)
        ));

        // 目录从中间截，文件名留全，截完放得下。
        let max = full * 0.6;
        let out = shorten_path(frame, path, size, max);
        assert!(
            out.starts_with("/pri") && out.ends_with("/Cargo.toml"),
            "{out}"
        );
        assert!(out.contains('…'), "{out}");
        assert!(
            frame.measure(FontKind::Ui, size, &out) <= max + 1e-3,
            "{out}"
        );

        // 连文件名都放不下：整个从中间截。
        let max = frame.measure(FontKind::Ui, size, "Cargo.toml");
        let out = shorten_path(frame, path, size, max);
        assert!(
            out.starts_with('/') && !out.ends_with("/Cargo.toml"),
            "{out}"
        );
    });
}

/// 回归：`fit` 按字符截，带肤色的 emoji、旗帜会被切开，留下半个。
#[test]
fn fit_never_splits_a_grapheme() {
    let mut renderer = Renderer::offscreen(10.0, 10.0, 2.0, Color::hex(0), None, None)
        .expect("测试机上有系统字体");
    renderer.draw(|frame| {
        let size = 13.0;
        let text = "👍🏽👍🏽👍🏽👍🏽e\u{301}e\u{301}e\u{301}";
        let full = frame.measure(FontKind::Ui, size, text);
        for step in 1..20 {
            let max = full * step as f32 / 20.0;
            for middle in [false, true] {
                let out = fit(frame, text, size, max, middle).into_owned();
                let (head, tail) = out.split_once('…').unwrap_or((&out, ""));
                // 截下的两段都是整个字素：开头在原文的字素边界上结束，结尾从字素边界开始。
                let bounds: Vec<usize> = text
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain([text.len()])
                    .collect();
                assert!(bounds.contains(&head.len()), "{out:?}");
                assert!(bounds.contains(&(text.len() - tail.len())), "{out:?}");
            }
        }
    });
}

/// 标签栏变窄时运行配置先省略名字，再窄时不显示；过长的名字截短。
#[test]
fn run_buttons_give_way_to_the_tabs() {
    use super::terminal::run_menu_width;
    let name = run_menu_width(50.0, 1000.0).unwrap();
    assert!(name > crate::theme::BUTTON);
    assert_eq!(
        run_menu_width(9999.0, 1000.0),
        run_menu_width(200.0, 1000.0)
    );
    assert_eq!(run_menu_width(50.0, 350.0), Some(crate::theme::BUTTON));
    assert_eq!(run_menu_width(50.0, 300.0), None);
    // 绘制时减去边距后仍然不窄于名字，不会把放得下的名字截短。
    for name in [24.0, 24.1, 24.3, 24.7, 31.333_334, 57.2, 99.99] {
        let room = run_menu_width(name, 1000.0).unwrap() - super::terminal::RUN_NAME_CHROME;
        assert!(room >= name, "{name}");
    }
}

#[test]
fn tooltip_lines_wrap_to_the_room_and_keep_continuations_in_the_body() {
    // 每个字符宽 10 逻辑像素。
    let measure = |s: &str| s.chars().count() as f32 * 10.0;
    assert_eq!(
        super::tip_lines("abcdef ghij\nsrc", 60.0, 0.0, measure),
        vec![("abcdef", true), ("ghij", true), ("src", false)]
    );
    // 快捷键只占第一行：第一段的宽度减去快捷键一栏，其余各段仍按整行宽度折行。
    assert_eq!(
        super::tip_lines("abc def\nabc def", 70.0, 20.0, measure),
        vec![("abc", true), ("def", true), ("abc def", false)]
    );
}
