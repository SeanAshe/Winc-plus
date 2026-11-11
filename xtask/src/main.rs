//! 开发自动化：生成测试样本、测量启动与内存、打包 macOS 应用。
//!
//! ```text
//! cargo xtask samples          生成测试样本到 target/samples/
//! cargo xtask bench [次数]      构建 release，测启动、内存、滚动，对照预算（默认启动测 20 次）
//! cargo xtask app [--target 目标] [--no-build]
//!                              构建 release，组成临时签名的 macOS 应用 winc.app；目标是
//!                              aarch64-apple-darwin 或 x86_64-apple-darwin，默认本机的架构
//! cargo xtask sparkle          取得固定版本的 Sparkle，输出 Sparkle 的工具目录（发布更新用）
//! cargo xtask linux-appcast 目录 地址 AppImage…
//!                              生成签名的 Linux 版 appcast（发布更新用）
//! cargo xtask windows-appcast 目录 地址 安装包…
//!                              生成签名的 Windows 版 appcast（发布更新用）
//! cargo xtask dmg              构建 release，打成 macOS 的 winc.app 和安装用的 dmg
//! cargo xtask dmg --app 路径    把签好名的 winc.app 原样做成 dmg，不重新构建、不重新签名
//!     [--output 路径]           指定最终 dmg 的路径
//! cargo xtask icons 目录        从应用的 SVG 生成打包用的 PNG 和 ICO
//! cargo xtask test             只测和 HEAD 相比改动涉及的 crate，测完清理过时的目标文件
//!     [--all] [--base 提交] [-- 测试名过滤等 cargo test 参数]
//! cargo xtask tidy             删除 target/debug/deps 中没有程序引用的 .rcgu.o
//! cargo xtask vim-fixtures     用本机的 vim 9.1 执行 Vim 模式的对照用例，写入录制文件
//!     [--check] [--full]        --check 只比较不写入；--full 把全部组合录制到 target 下
//!     [--vim 路径]              默认使用 /usr/bin/vim
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod bench;
#[cfg(unix)]
mod dev;
mod dmg;
mod feed;
#[cfg(test)]
mod file_types;
mod icons;
mod samples;
mod sparkle;
mod vim;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let result = match args.first().map(String::as_str) {
        Some("samples") => samples::samples().map(|_| true).map_err(|e| e.to_string()),
        Some("app") => app_options(&args[1..])
            .and_then(|(target, prebuilt)| dmg::app(target, prebuilt))
            .map(|(app, version)| {
                println!("{}（{version}）", app.display());
                true
            }),
        Some("sparkle") => sparkle::unpacked().map(|dir| {
            println!("{}", dir.join("bin").display());
            true
        }),
        Some("linux-appcast") => appcast(feed::Platform::Linux, &args, "AppImage"),
        Some("windows-appcast") => appcast(feed::Platform::Windows, &args, "安装包"),
        Some("dmg") => dmg_options(&args[1..])
            .and_then(|(app, output)| dmg::dmg(app, output))
            .map(|_| true),
        Some("icons") if args.len() == 2 => icons::write(Path::new(&args[1])).map(|_| true),
        #[cfg(unix)]
        Some("test") => dev::test(&args[1..]),
        #[cfg(unix)]
        Some("tidy") => dev::tidy().map(|()| true),
        Some("vim-fixtures") => vim::fixtures(&args[1..]),
        Some("bench") => {
            // 至少测一次，否则没有样本可排序、无法取出中位数。
            let runs = args
                .get(1)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(20)
                .max(1);
            bench::bench(runs)
        }
        _ => {
            eprintln!(
                "用法：cargo xtask samples | bench [次数] | app [--target 目标] [--no-build] | icons 目录 | sparkle | linux-appcast 目录 地址 AppImage… | windows-appcast 目录 地址 安装包… | dmg [--app 路径] [--output 路径] | test [--all] [--base 提交] [-- 参数] | tidy | vim-fixtures [--check] [--full] [--vim 路径]"
            );
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("xtask：{e}");
            ExitCode::FAILURE
        }
    }
}

/// 解析 `app` 的目标架构参数（`--target`）和是否跳过构建（`--no-build`）。
fn app_options(args: &[String]) -> Result<(Option<&str>, bool), String> {
    let mut target = None;
    let mut prebuilt = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--target" => target = Some(args.next().ok_or("--target 后面要跟目标")?.as_str()),
            "--no-build" => prebuilt = true,
            other => return Err(format!("app 不认识参数 {other}")),
        }
    }
    Ok((target, prebuilt))
}

/// 解析 `dmg` 的已有应用路径（`--app`）和输出路径（`--output`）。
fn dmg_options(args: &[String]) -> Result<(Option<&Path>, Option<&Path>), String> {
    let mut app = None;
    let mut output = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--app" => app = Some(Path::new(args.next().ok_or("--app 后面要跟路径")?)),
            "--output" => output = Some(Path::new(args.next().ok_or("--output 后面要跟路径")?)),
            other => return Err(format!("dmg 不认识参数 {other}")),
        }
    }
    Ok((app, output))
}

/// 按 `args`（命令、目录、下载地址、更新包…）生成签名的 appcast。
///
/// `package` 是用法说明里更新包一栏的叫法（`AppImage`、`安装包`）。
fn appcast(platform: feed::Platform, args: &[String], package: &str) -> Result<bool, String> {
    let (Some(out), Some(base), true) = (args.get(1), args.get(2), args.len() > 3) else {
        return Err(format!(
            "用法：cargo xtask {} <目录> <下载地址> <{package}>…",
            args[0]
        ));
    };
    let files: Vec<PathBuf> = args[3..].iter().map(PathBuf::from).collect();
    feed::write(platform, Path::new(out), base, &files).map(|path| {
        println!("{}", path.display());
        true
    })
}

/// 返回 workspace 根目录。
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 在 workspace 根目录下")
        .to_path_buf()
}

/// 构建 release 版的主程序 `winc-app` 和终端模块的辅助进程 `winc-pty`。
///
/// `targets` 为空时构建本机架构，否则构建列出的各个目标（如 `aarch64-apple-darwin`）。构建带
/// `--locked`，依赖版本与 `Cargo.lock` 一致。
///
/// # Errors
///
/// `cargo build` 启动失败或退出状态非零时返回错误描述。
fn build_release(targets: &[&str]) -> Result<(), String> {
    let status = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "winc",
            "-p",
            "winc-pty",
        ])
        .args(targets.iter().flat_map(|t| ["--target", t]))
        .current_dir(root())
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("release 构建失败".into());
    }
    Ok(())
}

/// 执行命令并等待进程结束。
///
/// # Errors
///
/// 命令无法启动或退出状态非零时返回错误描述。
fn run(command: &mut Command) -> Result<(), String> {
    let name = command.get_program().to_string_lossy().into_owned();
    let status = command
        .status()
        .map_err(|e| format!("无法运行 {name}：{e}"))?;
    if !status.success() {
        return Err(format!("{name} 失败（{status}）"));
    }
    Ok(())
}
