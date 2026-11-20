//! 测量：滚动 Git 日志时每帧的绘制耗时。

use super::*;

/// 测量：在 `WINC_MEASURE_REPO`（默认是本仓库）的日志里滚动，分别看当前分支和全部分支。
///
/// 运行方式：`cargo test -p winc-app --release measure_log_scroll -- --ignored --nocapture`。
#[test]
#[ignore = "测量，手动运行"]
fn measure_log_scroll() {
    use std::time::Instant;
    let repo = std::env::var_os("WINC_MEASURE_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let mut app = app_with(&repo, false);
    app.settings_toggle(Setting::GitModule);
    app.git_refresh();
    let frames = |app: &mut App, label: &str| {
        paint(app);
        // 一路滚到底：途中按需读取下一批提交，记下每帧的耗时，看平均和最慢的几帧。
        let mut times = Vec::new();
        for _ in 0..1200 {
            let start = Instant::now();
            app.editor.active_mut().unwrap().scroll_line += 1.7;
            app.clamp_scroll();
            app.poll_log();
            paint(app);
            times.push(start.elapsed());
        }
        let rows = app
            .editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .commits
            .len();
        let mean = times.iter().sum::<std::time::Duration>() / times.len() as u32;
        times.sort();
        let p95 = times[times.len() * 95 / 100];
        let worst = &times[times.len() - 3..];
        eprintln!("{label}（{rows} 个提交）: 平均 {mean:?}，p95 {p95:?}，最慢 {worst:?}");
    };
    app.run_action(Action::GitShowLog);
    frames(&mut app, "当前分支");
    app.change_log_query(|q| q.branches.clear());
    app.editor.active_mut().unwrap().scroll_line = 0.0;
    frames(&mut app, "全部分支");
}
