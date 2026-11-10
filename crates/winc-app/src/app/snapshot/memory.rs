//! 窗口内存的测试：在进程里建起一个窗口的全部状态（绘制到内存），每步后读进程的物理内存
//! （footprint），再建第二个窗口测量它额外占多少；单进程多窗口的设计按这些数字取舍。
//!
//! 手动运行：`WINC_MEMORY_REPO=<仓库的本地克隆> cargo test --release -p winc-app --lib --
//! measure_window_memory --ignored --nocapture`；打开的是该变量指向的仓库。
//! 不直接打开本仓库，因为它开在 winc 窗口里时请求会被交给那个窗口，测不到占用；未设置时跳过。

#![allow(unsafe_code)] // 读进程的物理内存（proc_pid_rusage）

use super::*;

/// 当前进程占用的物理内存（字节），与 `footprint` 命令输出的「Footprint」是同一个数。
fn footprint() -> u64 {
    // SAFETY: `rusage_info_v2` 只有整数字段，全零是合法的值。
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    // SAFETY: `info` 是 `rusage_info_v2` 大小的可写内存，`RUSAGE_INFO_V2` 让内核按这个结构填写。
    let ok = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V2,
            (&raw mut info).cast(),
        )
    };
    assert_eq!(ok, 0, "proc_pid_rusage");
    info.ri_phys_footprint
}

/// 按步骤记录物理内存的变化，结束时打印成一张表。
struct Steps {
    /// 上一次记录时的物理内存（字节），用来计算增量。
    last: u64,
    /// 上一次记录的时刻，用来计算该步的耗时。
    at: std::time::Instant,
    /// 每步一行：步骤名称、增量（字节）、累计物理内存（字节）、该步耗时（毫秒）。
    rows: Vec<(&'static str, i64, u64, f64)>,
}

impl Steps {
    fn new() -> Steps {
        Steps {
            last: footprint(),
            at: std::time::Instant::now(),
            rows: Vec::new(),
        }
    }

    /// 记录 `name` 这一步之后的物理内存、该步的增量与耗时。
    fn mark(&mut self, name: &'static str) {
        let now = footprint();
        let ms = self.at.elapsed().as_secs_f64() * 1000.0;
        self.rows
            .push((name, now as i64 - self.last as i64, now, ms));
        self.last = now;
        self.at = std::time::Instant::now();
    }

    fn print(&self) {
        let mb = |b: f64| b / (1024.0 * 1024.0);
        for (name, delta, total, ms) in &self.rows {
            println!(
                "{name:<40} {:>+8.1} MB {:>8.1} MB {ms:>8.1} ms",
                mb(*delta as f64),
                mb(*total as f64)
            );
        }
    }
}

/// 建起一个窗口的全部状态并逐步记录内存：渲染器（字体、字形缓存、整窗大小的画面）、打开仓库、
/// 打开文件、启用 Git 模块、绘制两帧。
///
/// `glyphs` 是同一进程内已有窗口的字体与字形缓存：第二个窗口复用它，与真实新开窗口的行为相同。
fn build_window(
    steps: &mut Steps,
    root: &Path,
    file: &Path,
    glyphs: Option<winc_render::SharedGlyphs>,
) -> App {
    let mut app = App::new(
        Editor::default(),
        None,
        Bench::from_env(),
        Config::default(),
    );
    steps.mark("App::new（空的界面状态）");

    // 先建 1×1 的离屏画面，只测量字体和空字形缓存；整窗画面单独测量：真实窗口的画面是
    // IOSurface，显示中的一块 16 MB，不在窗口之间共用。
    let renderer = Renderer::offscreen(1.0, 1.0, SCALE, app.palette.frame_bg, None, None).unwrap();
    app.renderer = Some(renderer);
    app.measure_fonts();
    app.renderer = None;
    let mut renderer =
        Renderer::offscreen(SIZE.0, SIZE.1, SCALE, app.palette.frame_bg, None, None).unwrap();
    if let Some(glyphs) = glyphs {
        renderer.use_glyphs(glyphs);
    }
    app.renderer = Some(renderer);
    steps.mark("渲染器：字体 + 整窗画面（2560×1640）");

    app.open_path(root);
    app.open_path(file);
    let lines = app.editor.active().map_or(0, |d| d.text().line_count());
    assert!(lines > 500, "app.rs 没有读进来：{lines} 行");
    assert_eq!(app.editor.root(), Some(root), "目录没有打开");
    steps.mark("打开仓库和 app.rs（文件树、文档）");

    app.settings_toggle(Setting::GitModule);
    assert!(
        app.git.status.is_some(),
        "Git 状态没有取到：{:?}",
        app.git.problem
    );
    steps.mark("Git 模块（状态、分支）");

    paint(&mut app);
    steps.mark("画两帧（字形缓存、布局）");
    for screen in 1..=20 {
        app.scroll_to_line(screen * 40, 0.0, false);
        paint(&mut app);
    }
    steps.mark("往下滚 20 屏（字形缓存涨满）");

    // 再模拟一段日常使用：打开多个文件、建立快速打开的文件索引、打开 Git 日志。
    let src = root.join("crates/winc-app/src");
    for name in [
        "view.rs",
        "layout.rs",
        "theme.rs",
        "keymap.rs",
        "main.rs",
        "instance.rs",
    ] {
        app.open_path(&src.join(name));
        paint(&mut app);
    }
    steps.mark("再开 6 个文件");
    app.open_picker();
    paint(&mut app);
    app.close_picker();
    steps.mark("快速打开的文件索引");
    app.run_action(crate::keymap::Action::GitShowLog);
    paint(&mut app);
    steps.mark("Git 日志");
    app
}

#[test]
#[ignore = "测量，手动运行"]
fn measure_window_memory() {
    let Some(root) = std::env::var_os("WINC_MEMORY_REPO").map(PathBuf::from) else {
        eprintln!("没有设置 WINC_MEMORY_REPO，跳过");
        return;
    };
    let file = root.join("crates/winc-app/src/app.rs");
    let mut steps = Steps::new();
    steps.mark("测试进程起点");

    let first = build_window(&mut steps, &root, &file, None);
    steps.mark("—— 第一个窗口建好");
    let glyphs = first.shared_glyphs();
    let second = build_window(&mut steps, &root, &file, glyphs);
    steps.mark("—— 第二个窗口建好");
    drop(first);
    steps.mark("关掉第一个窗口");
    drop(second);
    steps.mark("关掉第二个窗口");
    steps.print();
}
