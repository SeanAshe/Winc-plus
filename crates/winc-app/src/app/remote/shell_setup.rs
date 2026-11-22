//! 远端主机上的 shell 准备：连接后把集成脚本写到主机上并列出其上的 shell，终端据此启动。

use std::sync::{Mutex, PoisonError};

use winc_backend::Target;
use winc_backend::remote::{self, Exec};

use crate::app::terminal::shells::ShellChoice;

/// 把 shell 集成脚本写到远端的脚本：参数是成对的相对路径和内容，写到主目录下的
/// `winc_pty::scripts::REMOTE_DIR`。先写到旁边再改名替换：正在启动的 shell 不会读到写了一半的
/// 脚本。目录和文件只有用户本人可以访问。
const INSTALL_SHELL: &str = concat!(
    r#"umask 077; d=$HOME/$1; shift; while [ $# -gt 1 ]; do f=$d/$1; "#,
    r#"mkdir -p "${f%/*}" && printf %s "$2" > "$f.new" && mv -f "$f.new" "$f" || exit 1; "#,
    r#"shift 2; done"#,
);

/// 集成脚本已经写好的主机：在这些主机上开的终端注入 shell 集成。
static SHELL_READY: Mutex<Vec<Target>> = Mutex::new(Vec::new());

/// 各台主机上检测到的 shell，供终端的 shell 菜单列出。
static SHELLS: Mutex<Vec<(Target, Vec<ShellChoice>)>> = Mutex::new(Vec::new());

/// 写集成脚本、列出主机上的 shell 各自最长等待的时间：5 秒，与查询远端环境的时限相同，超时按
/// 没有写好、没有检测到处理。
const SHELLS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 正在写集成脚本、列出 shell 的主机，每个 [`Setup`] 一项；这些主机上的终端等完成后再启动。
static SHELL_SETUP: Mutex<Vec<Target>> = Mutex::new(Vec::new());

/// 为一台主机进行中的一次准备；析构时结束并调用 `done`，线程中途退出时同样执行。
pub(crate) struct Setup {
    target: Target,
    done: Option<Box<dyn Fn() + Send>>,
}

impl Setup {
    /// 记下开始为 `target` 准备；结束后调用 `done`。
    pub(crate) fn begin(target: &Target, done: Option<Box<dyn Fn() + Send>>) -> Self {
        let mut setup = SHELL_SETUP.lock().unwrap_or_else(PoisonError::into_inner);
        setup.push(target.clone());
        let target = target.clone();
        Self { target, done }
    }

    /// 同时写集成脚本、列出 shell，各一次往返；应在后台线程调用。
    pub(super) fn run(self) {
        std::thread::scope(|scope| {
            scope.spawn(|| install_shell_scripts(&self.target));
            detect_shells(&self.target);
        });
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let mut setup = SHELL_SETUP.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = setup.iter().position(|host| *host == self.target) {
            setup.swap_remove(at);
        }
        drop(setup);
        if let Some(done) = &self.done {
            done();
        }
    }
}

/// 是否还在为 `target` 写集成脚本、列出 shell。
pub(crate) fn shell_pending(target: &Target) -> bool {
    let setup = SHELL_SETUP.lock().unwrap_or_else(PoisonError::into_inner);
    setup.contains(target)
}

/// 测试中直接记下 `target` 上的集成脚本是否写好，代替经连接写入。
#[cfg(all(test, unix))]
pub(crate) fn set_shell_ready(target: &Target, written: bool) {
    let mut ready = SHELL_READY.lock().unwrap_or_else(PoisonError::into_inner);
    ready.retain(|known| known != target);
    if written {
        ready.push(target.clone());
    }
}

/// 返回 `target` 上检测到的 shell，登录 shell 排在最前；尚未检测完或检测失败时为空。
pub(crate) fn shells(target: &Target) -> Vec<ShellChoice> {
    let known = SHELLS.lock().unwrap_or_else(PoisonError::into_inner);
    let found = known.iter().find(|(host, _)| host == target);
    found.map(|(_, shells)| shells.clone()).unwrap_or_default()
}

/// 测试中直接记下 `target` 上的 shell，代替经连接检测；用到它的测试只在 Unix 上编译。
#[cfg(all(test, unix))]
pub(crate) fn set_shells(target: &Target, shells: Vec<ShellChoice>) {
    let mut known = SHELLS.lock().unwrap_or_else(PoisonError::into_inner);
    known.retain(|(host, _)| host != target);
    known.push((target.clone(), shells));
}

/// 列出 `target` 上的 shell 并记下来，一次往返；每次连接都重新检测。
fn detect_shells(target: &Target) {
    let mut exec = Exec::new("sh")
        .arg("-c")
        .arg(crate::app::terminal::shells::REMOTE_DETECT)
        .arg("winc");
    for path in crate::app::terminal::shells::remote_candidates() {
        exec = exec.arg(path);
    }
    let home = winc_backend::virtual_path(target, b"/");
    let deadline = std::time::Instant::now() + SHELLS_TIMEOUT;
    let output = remote::exec(&home, &exec, &|| std::time::Instant::now() >= deadline);
    let found = output.ok().map_or_else(Vec::new, |output| {
        crate::app::terminal::shells::remote_shells(&String::from_utf8_lossy(&output.stdout))
    });
    let mut known = SHELLS.lock().unwrap_or_else(PoisonError::into_inner);
    known.retain(|(host, _)| host != target);
    known.push((target.clone(), found));
}

/// 是否 `target` 上的 shell 集成脚本已经写好。
pub(crate) fn shell_ready(target: &Target) -> bool {
    let ready = SHELL_READY.lock().unwrap_or_else(PoisonError::into_inner);
    ready.contains(target)
}

/// 把 shell 集成脚本写到 `target` 上，一次往返；写不成（主目录只读）时那台主机上的终端不带
/// 集成，其余不受影响。脚本每次连接都重写：内容随本程序的版本变化。
fn install_shell_scripts(target: &Target) {
    let mut exec = Exec::new("sh")
        .arg("-c")
        .arg(INSTALL_SHELL)
        .arg("winc")
        .arg(winc_pty::scripts::REMOTE_DIR);
    for (name, content) in winc_pty::scripts::remote() {
        exec = exec.arg(name).arg(content.as_ref());
    }
    let home = winc_backend::virtual_path(target, b"/");
    let deadline = std::time::Instant::now() + SHELLS_TIMEOUT;
    let written = remote::exec(&home, &exec, &|| std::time::Instant::now() >= deadline)
        .is_ok_and(|output| output.code == Some(0));
    let mut ready = SHELL_READY.lock().unwrap_or_else(PoisonError::into_inner);
    ready.retain(|known| known != target);
    if written {
        ready.push(target.clone());
    }
}

#[cfg(test)]
mod tests;
