//! 各模块测试共用的辅助函数：在系统临时目录下为每个测试创建互不干扰的目录。

use std::path::PathBuf;

/// 在系统临时目录下新建一个空目录（已有的先清空），返回规范化后的路径（macOS 的临时目录位于
/// `/private` 之下，打开的文件记录的是真实路径）。
///
/// 目录名带调用处的源文件、`name` 和进程号，不同模块的测试用同一个 `name` 也不会撞在一起。
///
/// # Panics
///
/// 创建目录或规范化路径失败（例如临时目录不可写）时 panic。
#[track_caller]
pub(crate) fn scratch(name: &str) -> PathBuf {
    let file = std::panic::Location::caller().file();
    let module = file
        .rsplit_once("src/")
        .map_or(file, |(_, rest)| rest)
        .trim_end_matches(".rs")
        .replace(['/', '\\'], "-");
    let dir = std::env::temp_dir().join(format!(
        "winc-editor-{module}-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    winc_config::files::canonicalize(&dir).unwrap()
}

/// 在本进程内启动服务端并登记为到 `name` 的连接，返回连接目标：之后该目标的虚拟路径指向本机的
/// 同名路径，文件操作和命令都经连接执行。
///
/// # Panics
///
/// `name` 不是合法的连接目标、无法创建管道或握手失败时 panic。
#[cfg(unix)]
pub(crate) fn loopback(name: &str) -> winc_backend::Target {
    use winc_backend::remote;

    let (requests, to_server) = std::io::pipe().unwrap();
    let (replies, from_server) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let _ = remote::serve(requests, from_server);
    });
    let target = winc_backend::Target::new(name).unwrap();
    let connection = remote::Connection::over(replies, to_server).unwrap();
    remote::register(target.clone(), connection);
    target
}
