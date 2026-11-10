//! 浮层（⌘P、⇧⌘P、符号、⌃G 跳转条）的测试：焦点归属、快捷键关闭、点编辑区关闭、淡入淡出。

use super::*;
use crate::app::animation::{Card, Clock, DIALOG_IN, DIALOG_OUT};
use crate::keymap::Action;
use winit::keyboard::{Key, NamedKey};

fn opened(app: &App, which: Action) -> bool {
    match which {
        Action::GoToLine => app.goto_input.is_some(),
        _ => app.picker.open,
    }
}

fn owns_focus(app: &App, which: Action) -> bool {
    match which {
        Action::GoToLine => app.focus == Focus::Prompt,
        _ => app.focus == Focus::Picker,
    }
}

/// 浮层开着时依次按下键位表中的每个快捷键：快捷键把焦点移到其他位置（⌘F、⇧⌘F、⌃G、⌃Tab、
/// ⌘B 等）时，浮层随之关闭，不留下 Esc 无法关闭、回车也无法回到的浮层。
#[test]
fn a_dialog_closes_when_a_shortcut_takes_its_focus() {
    let dir = sample_folder("dialog-focus");
    let mut bad = Vec::new();
    for which in [
        Action::QuickOpen,
        Action::ShowCommands,
        Action::GoToSymbol,
        Action::GoToLine,
    ] {
        for b in crate::keymap::current() {
            if b.second.is_some() || matches!(b.action, Action::Quit | Action::CloseWindow) {
                continue;
            }
            let mut app = app_with(&dir.join("short.rs"), false);
            app.run_action(which);
            assert!(opened(&app, which), "{which:?}");
            app.on_shortcut(b.first);
            // 按键之后的下一帧。
            app.relayout();
            if opened(&app, which) && !owns_focus(&app, which) {
                bad.push(format!(
                    "{which:?} + {:?}: 焦点在 {:?}",
                    b.action, app.focus
                ));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// 跳转条开着时点击编辑区：跳转条关闭，焦点回到编辑区。
#[test]
fn clicking_the_editor_closes_the_go_to_bar() {
    let dir = sample_folder("dialog-click");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.open_goto();
    paint(&mut app);
    let e = app.layout.editor;
    crate::app::snapshot::click(&mut app, (e.x + e.w / 2.0, e.y + e.h / 2.0));
    paint(&mut app);
    assert_eq!(app.focus, Focus::Editor);
    assert!(app.goto_input.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

fn pixel(app: &App, (x, y): (f32, f32)) -> u32 {
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let (x, y) = ((x * SCALE) as usize, (y * SCALE) as usize);
    image.pixels[y * image.width as usize + x]
}

/// ⌘P 面板、跳转条、新建工作树对话框和窗口中画出来的提示框出现时整张淡入并放大，关闭后按
/// 关闭前的最后一帧淡出，与设置浮层共用同一套动画；淡出结束后丢弃保存的卡片。
#[test]
fn dialogs_fade_in_and_out_like_settings() {
    let dir = sample_folder("dialog-motion");
    for (card, open) in [
        (Card::Picker, Action::QuickOpen),
        (Card::Bar, Action::GoToLine),
        (Card::Dialog, Action::GitNewWorktree),
        (Card::Alert, Action::GitNewWorktree),
    ] {
        let mut app = app_with(&dir.join("short.rs"), false);
        paint(&mut app);
        if card == Card::Dialog {
            // 示例目录不是 Git 仓库，Git 模块也没有启用：直接打开对话框。
            app.git.status = Some(Default::default());
            app.open_worktree_dialog();
        } else if card == Card::Alert {
            let buttons = vec![
                crate::app::AlertButton::new("好"),
                crate::app::AlertButton::cancel("取消"),
            ];
            app.show_alert("提示", "", buttons, None, |_, _| None);
        } else {
            app.run_action(open);
        }
        paint(&mut app);
        let r = match card {
            Card::Picker => app.layout.picker,
            Card::Bar => crate::layout::goto_bar(app.layout.editor),
            Card::Dialog => app.dialog_layout().card,
            Card::Alert => app.alert.as_ref().unwrap().parts.buttons[0],
        };
        // 卡片左上角内侧的一点：底色与下方的编辑区不同。
        let probe = (r.x + 6.0, r.y + 6.0);
        let shown = pixel(&app, probe);
        assert!(app.card_fx(card).shot.is_some());

        app.card_fx(card).motion = Some((true, Clock::frozen(DIALOG_IN / 4, DIALOG_IN)));
        assert!(app.animating());
        paint(&mut app);
        let fading_in = pixel(&app, probe);
        assert_ne!(fading_in, shown, "{card:?} 还没完全出来");

        match card {
            Card::Picker => app.close_picker(),
            Card::Bar => app.close_prompt(),
            Card::Dialog => app.close_form_dialog(),
            Card::Alert => _ = app.alert_key(&Key::Named(NamedKey::Escape), false),
        }
        app.focus = Focus::Editor;
        paint(&mut app);
        let gone = pixel(&app, probe);
        app.card_fx(card).motion = Some((false, Clock::frozen(DIALOG_OUT / 2, DIALOG_OUT)));
        assert!(app.card_fx(card).closing().is_some());
        paint(&mut app);
        let fading_out = pixel(&app, probe);
        assert_ne!(fading_out, gone, "{card:?} 淡出时还看得到");

        app.card_fx(card).motion = Some((false, Clock::frozen(DIALOG_OUT, DIALOG_OUT)));
        app.settle_animations();
        assert!(app.card_fx(card).motion.is_none());
        assert!(app.card_fx(card).shot.is_none());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：⌘P 面板和跳转条出现到三分之一、消失到一半。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_dialog_motion() {
    let dir = sample_folder("dialog-motion-shot");
    let mut app = app_with(&dir.join("short.rs"), true);
    app.open_picker();
    paint(&mut app);
    app.picker.card.motion = Some((true, Clock::frozen(DIALOG_IN / 3, DIALOG_IN)));
    shoot(&mut app, "dialog-picker-in");
    app.picker.card.motion = None;
    app.close_picker();
    paint(&mut app);
    app.picker.card.motion = Some((false, Clock::frozen(DIALOG_OUT / 2, DIALOG_OUT)));
    app.open_goto();
    paint(&mut app);
    app.bar_card.motion = Some((true, Clock::frozen(DIALOG_IN / 3, DIALOG_IN)));
    app.picker.card.motion = Some((false, Clock::frozen(DIALOG_OUT / 2, DIALOG_OUT)));
    shoot(&mut app, "dialog-bar-in-picker-out");
    let _ = std::fs::remove_dir_all(&dir);
}

/// ⌃G、⌘P、⇧⌘P、⇧⌘O 再按一次时都关闭各自打开的浮层，焦点回到编辑区。
#[test]
fn pressing_the_shortcut_again_closes_the_dialog() {
    let dir = sample_folder("dialog-toggle");
    for which in [
        Action::QuickOpen,
        Action::ShowCommands,
        Action::GoToSymbol,
        Action::GoToLine,
    ] {
        let mut app = app_with(&dir.join("short.rs"), false);
        app.run_action(which);
        assert!(opened(&app, which), "{which:?}");
        app.run_action(which);
        assert!(!opened(&app, which), "{which:?} 再按一次");
        assert_eq!(app.focus, Focus::Editor, "{which:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
