//! 各 crate 的测试共用的辅助函数。

use std::path::Path;

/// 写入可执行的脚本 `path`，内容为 `text`。
///
/// Unix 上由子进程写入并设为可执行：本进程不持有文件的写入端，其他测试线程同时启动子进程时，
/// 执行该脚本不会得到 `ExecutableFileBusy`。
///
/// # Panics
///
/// 写入失败时 panic。
pub fn write_script(path: &Path, text: &str) {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("/bin/sh")
            .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
            .arg(path)
            .stdin(Stdio::piped())
            .spawn()
            .expect("启动写入脚本的 sh");
        let mut stdin = child.stdin.take().expect("sh 的标准输入");
        stdin.write_all(text.as_bytes()).expect("写入脚本内容");
        drop(stdin);
        assert!(
            child.wait().expect("等待 sh").success(),
            "写入 {}",
            path.display()
        );
    }
    #[cfg(not(unix))]
    std::fs::write(path, text).expect("写入脚本");
}
