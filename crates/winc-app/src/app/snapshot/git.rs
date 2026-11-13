//! Git 模块的测试：共用的仓库与应用搭建，各项功能的测试按功能分在 `git/` 下。

use std::process::Command;

use winc_editor::diff_view::{RIGHT, SideRow};
use winc_editor::git::{Change, Head};

use winit::keyboard::{Key, NamedKey};

use crate::app::git::Target;
use crate::app::git::commit::CommitGroup;
use crate::app::{ChangeTarget, CommitRow, CommitTarget, DiffTarget};
use crate::keymap::Action;

use super::*;

mod blame;
mod branch;
mod changes;
mod commit;
mod commit_selection;
mod diff;
mod log;
mod log_details;
mod merge;
mod module;
mod question;
mod rebase;
mod repository;
mod worktree;

/// 机器上没有可用的 git 时跳过该测试并返回 `true`。
///
/// 设置了 `CI` 环境变量时不允许跳过：没装 git 的环境里，Git 的测试会全部静默通过。
fn skip_without_git(reason: &str) -> bool {
    assert!(
        std::env::var_os("CI").is_none(),
        "CI 上必须能运行 git：{reason}"
    );
    eprintln!("跳过：{reason}");
    true
}

/// 打开模块后 git 不可用（未安装 git 或缺开发者工具）时跳过，跳过规则见 [`skip_without_git`]。
fn git_unusable(app: &App) -> bool {
    match &app.git.problem {
        Some(problem) => skip_without_git(problem),
        None => false,
    }
}

/// 在 `dir` 中创建只有一次提交的仓库，再修改 `src/main.rs` 并新增 `src/new.rs`。
///
/// 机器上没有可用的 git 时返回 `false`。
fn make_repo(dir: &Path) -> bool {
    let made = make_repo_inner(dir);
    if !made {
        skip_without_git("无法用 git 建测试仓库");
    }
    made
}

fn make_repo_inner(dir: &Path) -> bool {
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !git(&["init", "-q", "-b", "main"]) || !git(&["add", "."]) {
        return false;
    }
    // 仓库自己的配置：winc 提交时不依赖本机的全局身份和签名设置。
    for (key, value) in [
        ("user.name", "t"),
        ("user.email", "t@t"),
        ("commit.gpgsign", "false"),
    ] {
        git(&["config", key, value]);
    }
    if !git(&["commit", "-q", "-m", "first"]) {
        return false;
    }
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.join("src/new.rs"), "\n").unwrap();
    true
}

/// 在 `dir` 中创建 60 行的仓库，改一处、删一行，用于测试差异的折叠与跳转。
fn long_repo(dir: &Path) -> bool {
    let text: String = (0..60).map(|i| format!("let value_{i} = {i};\n")).collect();
    std::fs::write(dir.join("src/main.rs"), &text).unwrap();
    if !make_repo(dir) {
        return false;
    }
    let changed = text
        .replace("let value_5 = 5;", "let value_5 = 500;")
        .replace("let value_50 = 50;\n", "");
    std::fs::write(dir.join("src/main.rs"), changed).unwrap();
    true
}

/// 弹出 `command` 所在的菜单（分支的操作菜单、储藏那一行的菜单，或提交视图的「⋯」）并选中它；
/// 测试中没有系统菜单，选中即执行。
fn choose_in_branch_menu(app: &mut App, command: crate::app::MenuCommand) {
    use crate::app::MenuCommand::{GitBranch, GitStashOp};
    use crate::app::input::context_menu::MenuTarget;
    let target = match command {
        GitBranch(_, index) => MenuTarget::BranchOps(index),
        GitStashOp(_, index) => MenuTarget::StashRow(index),
        _ => MenuTarget::CommitMore,
    };
    app.open_context_menu(target, (0.0, 0.0));
    app.run_menu_command(command);
}

/// 按窗口内提示框的第 `button` 个按钮：第一个是默认按钮，最后一个是「取消」。
fn press_alert(app: &mut App, button: usize) {
    paint(app);
    let rect = app.overlays.alert.as_ref().unwrap().parts.buttons[button];
    app.on_mouse_move(rect.x + 4.0, rect.y + 4.0);
    app.alert_mouse_down();
}

/// 在测试仓库 `dir` 中运行 `git` 命令，失败时 panic。
fn git_in(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(ok, "git {args:?}");
}

/// 创建测试仓库、打开工作区并开启 Git 模块；git 不可用时返回 `None`（测试跳过）。
fn git_app(name: &str, long: bool) -> Option<(App, PathBuf)> {
    git_app_in(name, long, false)
}

/// 同 [`git_app`]，可选深色配色（截图用）。
fn git_app_in(name: &str, long: bool, dark: bool) -> Option<(App, PathBuf)> {
    let dir = search_folder(name);
    let made = if long {
        long_repo(&dir)
    } else {
        make_repo(&dir)
    };
    if !made {
        return None;
    }
    let mut app = app_with(&dir, dark);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return None;
    }
    let root = app.editor.root().unwrap().to_path_buf();
    Some((app, root))
}

/// 打开 Git 日志并改为显示全部分支，返回是否执行了打开日志的操作。
///
/// 日志默认只显示当前分支；使用本函数的测试检查包括其他分支在内的提交图。
fn show_all_log(app: &mut App) -> bool {
    let shown = app.run_action(Action::GitShowLog);
    app.change_log_query(|q| q.branch = None);
    shown
}

/// 日志用的仓库：主线上几次提交，一个功能分支分出去又合回来，一个标签，一个领先的分支，一个
/// 远程分支。
///
/// 提交时间从 `days` 天前起每次往后一天，作者轮换。
fn history_repo(root: &Path, days: i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let base = now - days * 86_400;
    let mut at = base;
    let mut step = 0;
    let mut commit = |file: &str, text: &str, message: &str| {
        std::fs::write(root.join(file), text).unwrap();
        let (name, email) = [
            ("张伟", "zhang@example.com"),
            ("Alice Chen", "alice@example.com"),
        ][step % 2];
        step += 1;
        at += 86_400;
        let date = format!("{at} +0800");
        let ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["add", "."])
            .output()
            .is_ok_and(|o| o.status.success())
            && Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["-c", &format!("user.name={name}"), "-c"])
                .arg(format!("user.email={email}"))
                .args(["commit", "-q", "-m", message])
                .env("GIT_AUTHOR_DATE", &date)
                .env("GIT_COMMITTER_DATE", &date)
                .output()
                .is_ok_and(|o| o.status.success());
        assert!(ok, "commit {message}");
    };
    commit(
        "src/config.rs",
        "pub struct Config;\n",
        "配置：读取用户目录下的配置文件",
    );
    commit("src/main.rs", "fn main() { run(); }\n", "启动时先加载配置");
    git_in(root, &["switch", "-q", "-c", "feature/login"]);
    commit("src/login.rs", "pub fn login() {}\n", "登录：账号密码表单");
    commit(
        "src/login.rs",
        "pub fn login() { check(); }\n",
        "登录：失败三次后锁定\n\n连续输错三次密码锁定十分钟，防止暴力破解。",
    );
    git_in(root, &["switch", "-q", "main"]);
    commit("README.md", "# demo\n\n使用说明\n", "补充使用说明");
    // 合并在第五次提交的那天下午。
    let date = format!("{} +0800", base + 5 * 86_400 + 43_200);
    let merged = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=张伟", "-c", "user.email=zhang@example.com"])
        .args([
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "合并分支 feature/login",
            "feature/login",
        ])
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(merged);
    git_in(root, &["tag", "v1.0"]);
    git_in(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    commit(
        "src/main.rs",
        "fn main() { run(); log(); }\n",
        "启动日志写到文件里",
    );
    git_in(root, &["switch", "-q", "-c", "dev"]);
    commit(
        "src/config.rs",
        "pub struct Config { debug: bool }\n",
        "配置：加入调试开关",
    );
    git_in(root, &["switch", "-q", "main"]);
}
