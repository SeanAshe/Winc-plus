//! 编辑器终端：移动时保留会话，复用文件标签的拖放和窗格，并可移回面板。

use super::*;
use crate::app::terminal::{HeaderHit, TermTab};
use crate::app::workspace::panes::TabDrop;
use winc_editor::panes::Axis;
use winc_pty::protocol::{Event, FromHelper, Line};

fn terminal_app(name: &str) -> (App, PathBuf, DocumentId) {
    let dir = sample_folder(name);
    let mut app = app_with(&dir.join("short.rs"), false);
    let file = app.editor.active().unwrap().id;
    app.config.terminal_enabled = Some(true);
    app.terminal.open = true;
    app.terminal.height = crate::app::terminal::DEFAULT_HEIGHT;
    app.relayout();
    let mut tab = TermTab::new(1, app.terminal_size());
    tab.name = "session".into();
    tab.rows[0] = Line {
        text: "retained output".into(),
        ..Line::default()
    };
    app.terminal.tabs.push(tab);
    app.focus = Focus::Terminal;
    paint(&mut app);
    (app, dir, file)
}

fn editor_tab(app: &App, document: DocumentId) -> Rect {
    app.hits
        .tabs
        .iter()
        .find(|hit| {
            app.editor
                .documents
                .get(hit.index)
                .is_some_and(|doc| doc.id == document)
        })
        .unwrap()
        .tab
}

fn drag(app: &mut App, from: Rect, to: (f32, f32)) {
    let start = (from.x + 12.0, from.y + from.h / 2.0);
    app.pointer.last_click = None;
    app.on_mouse_move(start.0, start.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(to.0, to.1);
    paint(app);
    app.on_mouse_up(MouseButton::Left);
    paint(app);
}

#[test]
fn terminal_editor_move_keeps_output_scrollback_and_split_layout() {
    let (mut app, dir, file) = terminal_app("terminal-editor-move");
    let size = app.terminal_size();
    crate::app::terminal::insert_pane(&mut app.terminal.tabs, 0, TermTab::new(2, size));
    app.terminal.tabs[0].share = 3.0;
    let first = &mut app.terminal.tabs[0].split.as_mut().unwrap().others[0];
    first.history = (0, 100);
    first.scroll = 3;
    first.scroll_frac = 0.25;
    first.cwd = b"/retained/cwd".to_vec();
    let shares = app.terminal.tab().unwrap().pane_shares();

    app.move_terminal_to_editor();
    paint(&mut app);
    let document = app.editor.active().unwrap();
    assert!(document.terminal && !document.shows_text() && !document.edits_text());
    let document = document.id;
    assert!(!app.terminal.open);
    assert_eq!(app.terminal.editor_document(), Some(document));
    assert_eq!(app.terminal.groups.len(), 1);
    assert_eq!(app.terminal.tab().unwrap().pane_shares(), shares);
    let retained = app
        .terminal
        .tab()
        .unwrap()
        .panes()
        .find(|tab| tab.id == 1)
        .unwrap();
    assert_eq!(retained.rows[0].text, "retained output");
    assert_eq!(
        (retained.history, retained.scroll, retained.scroll_frac),
        ((0, 100), 3, 0.25)
    );
    assert_eq!(retained.cwd, b"/retained/cwd");
    assert_eq!(app.terminal_view().y, app.layout.editor.y);

    app.move_editor_terminal_to_panel(document);
    paint(&mut app);
    assert!(app.editor.document(document).is_none());
    assert_eq!(app.editor.active().unwrap().id, file);
    assert!(app.terminal.open);
    assert_eq!(app.terminal.editor_document(), None);
    assert_eq!(app.terminal.tab().unwrap().pane_shares(), shares);
    let retained = app
        .terminal
        .tab()
        .unwrap()
        .panes()
        .find(|tab| tab.id == 1)
        .unwrap();
    assert_eq!(retained.rows[0].text, "retained output");
    assert_eq!(
        (retained.history, retained.scroll, retained.scroll_frac),
        ((0, 100), 3, 0.25)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_editor_drag_reorders_splits_and_returns_to_the_dock() {
    let (mut app, dir, file) = terminal_app("terminal-editor-drag");
    let panel_tab = app
        .hits
        .terminal
        .iter()
        .find(|(_, hit)| *hit == HeaderHit::Tab(0))
        .unwrap()
        .0;
    let body = app.layout.editor;
    drag(
        &mut app,
        panel_tab,
        (body.x + body.w / 2.0, body.y + body.h / 2.0),
    );
    let terminal = app.editor.active().unwrap().id;
    assert!(app.editor.active().unwrap().terminal);
    assert_eq!(app.terminal.tab().unwrap().id, 1);

    let source = editor_tab(&app, terminal);
    let target = editor_tab(&app, file);
    drag(
        &mut app,
        source,
        (target.x + 4.0, target.y + target.h / 2.0),
    );
    assert_eq!(
        app.editor
            .tabs()
            .iter()
            .map(|doc| doc.id)
            .collect::<Vec<_>>(),
        [terminal, file]
    );

    let source = editor_tab(&app, terminal);
    let body = app.layout.editor;
    drag(
        &mut app,
        source,
        (body.right() - 12.0, body.y + body.h / 2.0),
    );
    assert!(app.editor.panes.is_split());
    assert_eq!(
        app.editor
            .tabs()
            .iter()
            .map(|doc| doc.id)
            .collect::<Vec<_>>(),
        [terminal]
    );
    assert!(
        app.editor
            .panes
            .others
            .iter()
            .any(|pane| pane.tabs == [file])
    );
    assert_eq!(app.terminal.tab().unwrap().id, 1);

    let source = editor_tab(&app, terminal);
    let start = (source.x + 12.0, source.y + source.h / 2.0);
    app.pointer.last_click = None;
    app.on_mouse_move(start.0, start.1);
    app.on_mouse_down(MouseButton::Left);
    let body = app.layout.editor;
    app.on_mouse_move(body.x + body.w / 2.0, body.bottom() - 2.0);
    paint(&mut app);
    assert!(app.terminal.open);
    assert!(app.terminal.want.is_empty());
    let panel = app.layout.terminal;
    app.on_mouse_move(panel.x + panel.w / 2.0, panel.y + panel.h / 2.0);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert!(!app.editor.panes.is_split());
    assert!(app.editor.document(terminal).is_none());
    assert_eq!(app.editor.active().unwrap().id, file);
    assert_eq!(app.terminal.tab().unwrap().id, 1);
    assert_eq!(app.terminal.tab().unwrap().rows[0].text, "retained output");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_editor_hidden_tabs_keep_messages_without_claiming_visible_frames() {
    let (mut app, dir, file) = terminal_app("terminal-editor-hidden");
    app.move_terminal_to_editor();
    let terminal = app.editor.active().unwrap().id;
    app.activate_tab(app.editor.index_of(file).unwrap());
    let blocks = FromHelper::Blocks {
        tab: 1,
        blocks: Vec::new(),
        early: false,
    };
    assert!(!app.terminal.message_is_shown(&blocks));
    app.terminal_message(FromHelper::Event {
        tab: 1,
        event: Event::Title("background title".into()),
    });
    app.terminal.tabs[0].name.clear();
    paint(&mut app);
    assert_eq!(
        app.editor.document(terminal).unwrap().name(),
        "background title"
    );
    app.activate_tab(app.editor.index_of(terminal).unwrap());
    assert!(app.terminal.message_is_shown(&blocks));
    assert_eq!(app.focus, Focus::Terminal);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_editor_close_removes_only_its_session_and_native_tab() {
    let (mut app, dir, file) = terminal_app("terminal-editor-close");
    app.terminal.tabs.push(TermTab::new(2, app.terminal_size()));
    app.move_terminal_to_editor();
    let terminal = app.editor.active().unwrap().id;
    app.close_tab(app.editor.index_of(terminal).unwrap());
    paint(&mut app);
    assert!(app.editor.document(terminal).is_none());
    assert_eq!(app.editor.active().unwrap().id, file);
    assert_eq!(
        app.terminal
            .all_tabs()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        [2]
    );
    assert!(
        app.terminal
            .groups
            .iter()
            .all(|group| group.editor.is_none())
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_editor_invalid_drop_and_cancellation_leave_the_session_in_place() {
    let (mut app, dir, _) = terminal_app("terminal-editor-cancel");
    app.move_terminal_to_editor();
    paint(&mut app);
    let terminal = app.editor.active().unwrap().id;
    let source = editor_tab(&app, terminal);
    let outside = (app.layout.window.right() + 100.0, -40.0);
    drag(&mut app, source, outside);
    assert_eq!(app.terminal.editor_document(), Some(terminal));
    assert_eq!(app.terminal.tab().unwrap().id, 1);
    assert!(!app.terminal.open);

    let source = editor_tab(&app, terminal);
    app.pointer.last_click = None;
    app.on_mouse_move(source.x + 12.0, source.y + source.h / 2.0);
    app.on_mouse_down(MouseButton::Left);
    let body = app.layout.editor;
    app.on_mouse_move(body.x + body.w / 2.0, body.bottom() - 2.0);
    assert!(app.terminal.open);
    app.cancel_tab_drag();
    paint(&mut app);
    assert!(!app.terminal.open);
    assert_eq!(app.terminal.editor_document(), Some(terminal));
    assert_eq!(app.terminal.tab().unwrap().id, 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_editor_alt_drag_moves_a_single_live_session_instead_of_duplicating_it() {
    let (mut app, dir, file) = terminal_app("terminal-editor-alt");
    app.move_terminal_to_editor();
    paint(&mut app);
    let terminal = app.editor.active().unwrap().id;
    let source = app.editor.panes.focused;
    app.pointer.modifiers = winit::keyboard::ModifiersState::ALT;
    app.drop_tabs(
        &[terminal],
        terminal,
        TabDrop::Split {
            pane: source,
            axis: Axis::Row,
            before: false,
        },
        true,
    );
    paint(&mut app);
    assert_eq!(
        app.editor
            .tabs()
            .iter()
            .map(|doc| doc.id)
            .collect::<Vec<_>>(),
        [terminal]
    );
    let other = app
        .editor
        .panes
        .others
        .iter()
        .find(|pane| pane.id == source)
        .unwrap();
    assert_eq!(other.tabs, [file]);
    assert_eq!(
        app.terminal
            .all_tabs()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        [1]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

fn input_region_pixels(app: &App, rect: Rect) -> Vec<u32> {
    let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
    let left = (rect.x * SCALE).round() as usize;
    let top = (rect.y * SCALE).round() as usize;
    let width = (rect.w * SCALE).round() as usize;
    let height = (rect.h * SCALE).round() as usize;
    let stride = image.width as usize;
    let mut pixels = Vec::with_capacity(width * height);
    for row in top..top + height {
        pixels.extend_from_slice(&image.pixels[row * stride + left..row * stride + left + width]);
    }
    pixels
}

#[test]
fn terminal_editor_composition_is_visible_only_in_the_focused_session() {
    let (mut app, dir, _) = terminal_app("terminal-editor-ime-owner");
    let size = app.terminal_size();
    app.terminal.tabs.push(TermTab::new(2, size));
    app.move_terminal_to_editor();
    paint(&mut app);
    let panel = app.terminal.panel_groups().next().unwrap();
    let region = |app: &App| {
        let grid = app.terminal_grid();
        Rect::new(grid.x, grid.y, 240.0, app.fonts.terminal_line_height + 2.0)
    };
    let editor_region = region(&app);
    let panel_region = app.with_terminal_group(panel, |app| region(app));
    let editor_before = input_region_pixels(&app, editor_region);
    let panel_before = input_region_pixels(&app, panel_region);
    app.on_ime(winit::event::Ime::Preedit("EDITOR_ONLY".into(), None));
    paint(&mut app);
    assert_eq!(input_region_pixels(&app, panel_region), panel_before);
    assert_ne!(input_region_pixels(&app, editor_region), editor_before);

    app.on_ime(winit::event::Ime::Preedit(String::new(), None));
    app.focus_terminal_group(panel);
    paint(&mut app);
    let editor_before = input_region_pixels(&app, editor_region);
    let panel_before = input_region_pixels(&app, panel_region);
    app.on_ime(winit::event::Ime::Preedit("PANEL_ONLY".into(), None));
    paint(&mut app);
    assert_eq!(input_region_pixels(&app, editor_region), editor_before);
    assert_ne!(input_region_pixels(&app, panel_region), panel_before);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_editor_relocation_hover_shows_tooltips_for_both_destinations() {
    let (mut app, dir, _) = terminal_app("terminal-editor-hover");
    let editor = app
        .hits
        .terminal
        .iter()
        .find(|(_, hit)| *hit == HeaderHit::Editor)
        .unwrap()
        .0;
    app.on_mouse_move(editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);
    if let Some(since) = app.overlays.tooltip.since {
        app.overlays.tooltip.since = since.checked_sub(std::time::Duration::from_secs(5));
    }
    app.tick();
    paint(&mut app);
    assert!(app.overlays.tooltip.shown);
    assert_eq!(crate::view::tests::tooltip(&app).unwrap().0, editor);

    app.move_terminal_to_editor();
    paint(&mut app);
    let panel = app.layout.terminal_return;
    app.on_mouse_move(panel.x + panel.w / 2.0, panel.y + panel.h / 2.0);
    if let Some(since) = app.overlays.tooltip.since {
        app.overlays.tooltip.since = since.checked_sub(std::time::Duration::from_secs(5));
    }
    app.tick();
    paint(&mut app);
    assert!(app.overlays.tooltip.shown);
    assert_eq!(crate::view::tests::tooltip(&app).unwrap().0, panel);
    std::fs::remove_dir_all(dir).unwrap();
}
