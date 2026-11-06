//! 目录监视的测试：只上报仓库状态文件之外的变化、变化过多时改为重扫、隐藏目录的变化被丢弃、附属
//! 工作树能看到主仓库 `.git` 的变化。

use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;
use crate::test_util::scratch;

/// 等待通知送达 `want` 中的全部路径（系统发送通知有延迟，最长等 10 秒），返回累积的所有路径。
fn wait_for(watcher: &Watcher, woken: &mpsc::Receiver<()>, want: &[&Path]) -> Vec<PathBuf> {
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !want.iter().all(|w| seen.iter().any(|p: &PathBuf| p == w)) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            woken.recv_timeout(left).is_ok(),
            "没等到 {want:?}，只有 {seen:?}"
        );
        seen.extend(watcher.take().paths);
    }
    seen
}

#[test]
fn only_repository_state_files_count_as_git_changes() {
    let root = Path::new("/r");
    for yes in [
        ".git/HEAD",
        ".git/index",
        ".git/packed-refs",
        ".git/refs/heads/main",
        ".git/worktrees/feat/HEAD",
        ".git/worktrees/feat/index",
    ] {
        assert!(git_state(&root.join(yes)), "{yes}");
    }
    for no in [
        ".git/index.lock",
        ".git/refs/heads/main.lock",
        ".git/objects/ab/cdef",
        ".git/HEAD/x",
        ".git/worktrees/feat/index.lock",
        ".git/worktrees/feat/logs/HEAD",
        "src/HEAD",
    ] {
        assert!(!git_state(&root.join(no)), "{no}");
    }
}

/// 公共 Git 目录在打开的目录之外时（附属工作树）：提交、分支的变化也要报告，那里的其他文件不算
/// 打开的目录里的变化。
#[test]
fn a_git_directory_outside_the_folder_is_watched() {
    let dir = scratch("linked");
    let git = dir.join("main/.git");
    fs::create_dir_all(git.join("refs/heads")).unwrap();
    fs::create_dir_all(git.join("objects")).unwrap();
    fs::create_dir_all(git.join("worktrees/feat")).unwrap();
    let linked = dir.join("feat");
    fs::create_dir_all(&linked).unwrap();

    let (sender, woken) = mpsc::channel();
    let mut watcher = Watcher::start(&linked, move || {
        let _ = sender.send(());
    })
    .unwrap();
    watcher.watch_git_dir(Some(&git));
    fs::write(linked.join("ready.txt"), "x").unwrap();
    wait_for(&watcher, &woken, &[&linked.join("ready.txt")]);
    while woken.recv_timeout(Duration::from_millis(300)).is_ok() {
        watcher.take();
    }

    fs::write(git.join("objects/pack"), "x").unwrap();
    fs::write(git.join("refs/heads/main"), "x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut changes = watcher.take();
    while !changes.git && Instant::now() < deadline {
        let _ = woken.recv_timeout(Duration::from_millis(200));
        let next = watcher.take();
        changes.git |= next.git;
        changes.paths.extend(next.paths);
    }
    assert!(changes.git);
    assert!(changes.paths.is_empty(), "{:?}", changes.paths);

    // 打开的目录里的 `.git` 本来就在监视之内，不另外监视。
    watcher.watch_git_dir(Some(&linked.join(".git")));
    assert!(watcher.pending.lock().unwrap().git_dir.is_none());

    let _ = fs::remove_dir_all(&dir);
}

/// 监视线程一边送通知、界面线程一边改监视的目录时不会互相等待：系统监视器处理改动时要等监视线程
/// 回复，而监视线程送通知时要拿累积变化的锁，改动期间不能拿着这把锁。
#[test]
fn changing_the_git_directory_while_events_arrive_does_not_deadlock() {
    let dir = scratch("watch-deadlock");
    let (folder, git) = (dir.join("folder"), dir.join("main/.git"));
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(git.join("refs")).unwrap();
    let mut watcher = Watcher::start(&folder, || {}).unwrap();

    let writing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let writer = {
        let (writing, folder) = (Arc::clone(&writing), folder.clone());
        std::thread::spawn(move || {
            let mut i = 0u32;
            while writing.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = fs::write(folder.join(format!("{}.txt", i % 8)), "x");
                i += 1;
            }
        })
    };
    let (done, finished) = mpsc::channel();
    let git_dir = git.clone();
    std::thread::spawn(move || {
        for _ in 0..200 {
            watcher.watch_git_dir(Some(&git_dir));
            watcher.watch_git_dir(None);
        }
        let _ = done.send(());
    });
    let result = finished.recv_timeout(Duration::from_secs(30));
    writing.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = writer.join();
    assert!(result.is_ok(), "改监视的目录时卡住了");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn changes_inside_the_folder_are_reported() {
    let dir = scratch("events");
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(dir.join("old.txt"), "x").unwrap();

    let (sender, woken) = mpsc::channel();
    let watcher = Watcher::start(&dir, move || {
        let _ = sender.send(());
    })
    .unwrap();
    // 监视器开始送通知之前的改动可能收不到：先改一个文件并等它送达，确认通知已就绪后再开始。
    fs::write(dir.join("ready.txt"), "x").unwrap();
    wait_for(&watcher, &woken, &[&dir.join("ready.txt")]);

    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/new.rs"), "fn main() {}").unwrap();
    fs::write(dir.join("src/new.rs"), "fn main() { }").unwrap();
    fs::rename(dir.join("old.txt"), dir.join("renamed.txt")).unwrap();
    fs::write(dir.join(".git/index"), "x").unwrap();

    let want = [dir.join("old.txt"), dir.join("renamed.txt")];
    let want: Vec<&Path> = want.iter().map(PathBuf::as_path).collect();
    let seen = wait_for(&watcher, &woken, &want);
    // 新目录中的文件：报文件本身，或者报新建的目录（Linux 的 inotify 要等目录建好以后才开始
    // 监视它，随即写入的文件只体现为该目录的新建；刷新该目录时文件一并出现）。
    let mut seen = seen;
    let (file, folder) = (dir.join("src/new.rs"), dir.join("src"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !seen.iter().any(|p| *p == file || *p == folder) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            woken.recv_timeout(left).is_ok(),
            "没等到 {file:?}，只有 {seen:?}"
        );
        seen.extend(watcher.take().paths);
    }
    // `.git` 内的变化不应上报。
    assert!(
        !seen.iter().any(|p| p.starts_with(dir.join(".git"))),
        "{seen:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn too_many_changes_become_a_rescan() {
    let dir = scratch("overflow");
    let (sender, woken) = mpsc::channel();
    let watcher = Watcher::start(&dir, move || {
        let _ = sender.send(());
    })
    .unwrap();
    fs::write(dir.join("ready.txt"), "x").unwrap();
    wait_for(&watcher, &woken, &[&dir.join("ready.txt")]);
    // 写一个文件可能先后报几次（Windows 上新建、修改各一次）：等晚到的都收完，下面收到的唤醒才是
    // `one.txt` 引起的。
    while woken.recv_timeout(Duration::from_millis(300)).is_ok() {
        watcher.take();
    }

    // 直接向累积的集合写入超过 `MAX_PATHS` 个路径，不必真的改动数千个文件。
    {
        let mut pending = watcher.pending.lock().unwrap();
        pending
            .paths
            .extend((0..=MAX_PATHS).map(|i| dir.join(i.to_string())));
    }
    fs::write(dir.join("one.txt"), "x").unwrap();
    woken.recv_timeout(Duration::from_secs(10)).unwrap();
    let changes = watcher.take();
    assert!(changes.rescan);
    assert!(changes.paths.len() <= 1, "{}", changes.paths.len());

    // 取走后状态清空，下一批变化会重新唤醒界面线程。`one.txt` 的通知可能晚到一次（Windows 上新建、
    // 修改各一次），除此之外不留下任何内容。
    let rest = watcher.take();
    assert!(!rest.rescan);
    assert!(!rest.git);
    assert!(
        rest.paths.iter().all(|p| p.ends_with("one.txt")),
        "{:?}",
        rest.paths
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hidden_folders_are_dropped() {
    let root = Path::new("/p");
    assert!(hidden(root, Path::new("/p/.git/index")));
    assert!(hidden(root, Path::new("/p/a/__pycache__/x.pyc")));
    assert!(!hidden(root, Path::new("/p/.github/ci.yml")));
    assert!(!hidden(root, Path::new("/p/src/main.rs")));
}
