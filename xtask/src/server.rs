//! 构建随安装包发布的远端服务进程（`winc-server`）。
//!
//! 服务进程运行在 Linux 主机上，与安装包所在的系统无关：每个安装包都带上各个架构的程序，连接时
//! 按远端主机的系统上传对应的一个。程序静态链接 musl，不依赖远端的系统库；没有 C 代码，用 Rust
//! 自带的链接器即可在任何系统上构建。

use std::path::PathBuf;
use std::process::Command;

use crate::{root, run};

/// 各个架构的构建目标，以及程序在安装包中的文件名。
///
/// 文件名的后缀是该系统上 `uname -s` 与 `uname -m` 的输出以 `-` 相连，连接时按它选取。
pub const PROGRAMS: [(&str, &str); 2] = [
    ("x86_64-unknown-linux-musl", "winc-server-Linux-x86_64"),
    ("aarch64-unknown-linux-musl", "winc-server-Linux-aarch64"),
];

/// 构建好的程序所在的目录，打包脚本从这里取用。
pub fn dir() -> PathBuf {
    root().join("target/remote-server")
}

/// 构建各个架构的服务进程，放进 [`dir`]，返回该目录。
///
/// # Errors
///
/// 无法安装构建目标、构建失败或复制失败时返回错误描述。
pub fn build() -> Result<PathBuf, String> {
    let out = dir();
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for (target, name) in PROGRAMS {
        run(Command::new("rustup").args(["target", "add", target]))?;
        let linker = format!(
            "CARGO_TARGET_{}_LINKER",
            target.to_uppercase().replace('-', "_")
        );
        run(Command::new(env!("CARGO"))
            .args(["build", "--release", "--locked", "-p", "winc-server"])
            .args(["--target", target])
            .env(linker, "rust-lld")
            .current_dir(root()))?;
        let built = root()
            .join("target")
            .join(target)
            .join("release/winc-server");
        std::fs::copy(&built, out.join(name))
            .map_err(|e| format!("无法复制 {}：{e}", built.display()))?;
    }
    Ok(out)
}
