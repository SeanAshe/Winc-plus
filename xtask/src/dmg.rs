//! 打包（`cargo xtask dmg`）：macOS 的 winc.app 和安装用的 dmg，含应用图标、dmg 窗口的背景和布局。

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::{build_release, root, run, sparkle};

#[cfg(unix)]
mod finder;

/// 应用的标识。
///
/// 系统按这个标识区分应用（权限、默认打开方式、Launch Services）。
const BUNDLE_ID: &str = "com.deeix.winc";

/// 构建 release，组成临时签名（ad-hoc）的 `target/release/bundle/winc.app`，返回应用的路径和
/// 版本号。
///
/// 应用包里有主程序、终端的辅助进程 `winc-pty`、应用内更新用的 `Sparkle.framework` 和图标。
/// `target` 是 Rust 的目标（`aarch64-apple-darwin` 或 `x86_64-apple-darwin`，在 Apple 芯片上
/// 交叉编译 Intel 版）；为 `None` 时构建本机架构。Apple 芯片和 Intel 各出一个安装包，各有自己的
/// appcast。appcast 的 `SUFeedURL` 由 `sparkle::UpdateConfig::plist_entries` 按架构生成；
/// 分发前由发布者用 Developer ID 重新签名。
///
/// # Errors
///
/// 不在 macOS 上、目标不认识、构建或签名失败时返回原因。
pub fn app(target: Option<&str>, prebuilt: bool) -> Result<(PathBuf, String), String> {
    if !cfg!(target_os = "macos") {
        return Err("应用包只能在 macOS 上做".into());
    }
    let arch = match target {
        None => mac_arch(std::env::consts::ARCH),
        Some("aarch64-apple-darwin") => "arm64",
        Some("x86_64-apple-darwin") => "x86_64",
        Some(other) => {
            return Err(format!(
                "不认识的目标 {other}（aarch64-apple-darwin 或 x86_64-apple-darwin）"
            ));
        }
    };
    if !prebuilt {
        build_release(target.as_slice())?;
    }
    let io = |e: std::io::Error| e.to_string();
    let release = root().join("target/release");
    // 指定了目标时，程序在 target/<目标>/release 下。
    let built = target.map_or_else(
        || release.clone(),
        |t| root().join("target").join(t).join("release"),
    );
    let bundle = built.join("bundle");
    let version = format!(
        "{}.{}",
        env!("CARGO_PKG_VERSION"),
        include_str!("../release/fork-version").trim()
    );
    let app = bundle.join("winc.app");
    let _ = std::fs::remove_dir_all(&bundle);
    let contents = app.join("Contents");
    std::fs::create_dir_all(contents.join("MacOS")).map_err(io)?;
    std::fs::create_dir_all(contents.join("Resources")).map_err(io)?;
    // 终端模块的辅助进程和主程序放在一起，主程序按自身旁边的位置查找辅助进程。
    let helper = contents.join("MacOS/winc-pty");
    std::fs::copy(built.join("winc"), contents.join("MacOS/winc")).map_err(io)?;
    std::fs::copy(built.join("winc-pty"), &helper).map_err(io)?;
    // 主程序不链接 Sparkle，运行时从 `Contents/Frameworks` 加载；没有更新配置时也带着。
    sparkle::embed(&contents)?;
    let updates = sparkle::UpdateConfig::load();
    std::fs::write(
        contents.join("Info.plist"),
        info_plist(&version, &updates, arch),
    )
    .map_err(io)?;
    std::fs::write(contents.join("PkgInfo"), "APPL????").map_err(io)?;
    // 图标必须在签名之前放进去：签名会封存 Resources 里的全部文件。
    let icon = contents.join("Resources").join(format!("{ICON_NAME}.icns"));
    app_icon(&bundle, &icon)?;

    // 可执行文件先单独签，应用的签名才包含它们（否则校验报未签名的子组件）。框架保留发布包
    // 自带的签名。
    run(Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&helper))?;
    run(Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&app))?;
    run(Command::new("codesign")
        .args(["--verify", "--strict"])
        .arg(&app))?;
    if !updates.enabled() {
        println!(
            "没有配置更新通道和公钥（WINC_UPDATE_CHANNELS、WINC_UPDATE_PUBLIC_KEY）：这个应用包不检查更新"
        );
    }
    Ok((app, version.to_owned()))
}

/// 返回 Rust 的架构名 `arch`（`std::env::consts::ARCH`）在 macOS 上的叫法：`arm64` 或 `x86_64`。
fn mac_arch(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        other => other,
    }
}

/// 返回安装包文件名中的架构：`arm64` 或 `x86_64`（按 `lipo -archs` 的结果）；主程序同时含两种
/// 架构时为 `universal`。
///
/// # Errors
///
/// `lipo` 运行失败或退出状态非零时返回原因。
pub fn app_arch(app: &Path) -> Result<String, String> {
    let output = Command::new("lipo")
        .arg("-archs")
        .arg(app.join("Contents/MacOS/winc"))
        .output()
        .map_err(|e| format!("无法运行 lipo：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "lipo 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let archs: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    Ok(match archs.as_slice() {
        [single] => single.clone(),
        _ => "universal".to_owned(),
    })
}

/// dmg 的卷名，也是打开后窗口的标题。
const VOLUME_NAME: &str = "winc";
/// dmg 窗口内容区的宽和高（点）。
///
/// 取自调参：640 点宽的内容区里并排放两个图标及其名字，400 点高容纳图标、名字和箭头。
const WINDOW: (u32, u32) = (640, 400);
/// 标题栏的高度（点）。
///
/// 窗口大小记录含标题栏，所以写进去的高度要加上标题栏高度。该值随系统版本变化，
/// 这里按 winc 支持的系统版本的标准窗口标题栏高度取 28 点。
#[cfg(unix)]
const TITLE_BAR: u32 = 28;
/// 背景图的高度（点）：内容区高度加上 80 点余量。各版本系统的标题栏高度不同，
/// 内容区以外的部分也需要背景，因此比 `WINDOW` 高。
const BACKGROUND_HEIGHT: u32 = 480;
/// 窗口左上角在屏幕上的位置（横、纵坐标，点）；超出屏幕时会被移回可见范围。
#[cfg(unix)]
const WINDOW_AT: (u32, u32) = (200, 120);
/// 图标的大小（点）；两个图标中心相距 320 点，图标加上名字的宽度小于该间距。
#[cfg(unix)]
const ICON_SIZE: f64 = 128.0;
/// 图标下面名字的字号（点）。
#[cfg(unix)]
const TEXT_SIZE: f64 = 12.0;
/// winc.app 的图标中心（内容区坐标，点）。
///
/// 比内容区的正中稍高，连同下面的名字整体在视觉上居中。
const APP_AT: (u32, u32) = (160, 186);
/// 「应用程序」替身的图标中心；背景上的箭头画在两个图标正中间。
const APPLICATIONS_AT: (u32, u32) = (480, 186);
/// 构建 release 版 `winc.app` 和 `.dmg`，只在 macOS 上可用。
///
/// 签名是本机临时签名（ad-hoc），没有公证，从网上下载的副本首次打开需要用户放行。`signed` 是
/// 已经用 Developer ID 签好名的 winc.app：给出时不构建、不重新签名，校验后原样装进 dmg。
///
/// # Errors
///
/// 构建失败、`signed` 的签名无效、打包用的系统工具执行失败时返回错误描述。
pub fn dmg(signed: Option<&Path>, output: Option<&Path>) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("dmg 只能在 macOS 上打".into());
    }

    let release = root().join("target/release");
    let io = |e: std::io::Error| e.to_string();
    let bundle = release.join("bundle");

    let (app, version) = match signed {
        Some(app) => {
            run(Command::new("codesign")
                .args(["--verify", "--strict", "--deep"])
                .arg(app))?;
            (app.to_path_buf(), bundle_version(app)?)
        }
        None => app(None, false)?,
    };
    let arch = app_arch(&app)?;
    // 卷图标用应用里的图标。
    let icon = app
        .join("Contents/Resources")
        .join(format!("{ICON_NAME}.icns"));

    // dmg：先做一个可写的空白镜像，挂载后放入内容、写好窗口布局；卸载后压到最小，
    // 再转成压缩的只读镜像。窗口布局里的背景图要按挂载着的卷上的文件编号来指，所以不能事先像
    // 普通文件那样放好再整个做成镜像。
    let work = bundle.join("dmg-work");
    std::fs::create_dir_all(&work).map_err(io)?;
    let background = work.join("background.tiff");
    dmg_background(&work, &background)?;

    let writable = work.join("writable.dmg");
    // 留出余量放背景图、卷图标和文件系统自身的开销，空着的部分压缩后几乎不占空间。
    let megabytes = dir_size(&app).map_err(io)? / 1_048_576 + 16;
    run(Command::new("hdiutil")
        .args(["create", "-quiet", "-fs", "HFS+", "-volname", VOLUME_NAME])
        .args(["-size", &format!("{megabytes}m")])
        .arg(&writable))?;
    // 挂到自己的目录上，不进 `/Volumes`：不和已经打开的 winc 镜像重名，也不出现在系统的卷列表中。
    let volume = work.join("volume");
    std::fs::create_dir_all(&volume).map_err(io)?;
    run(Command::new("hdiutil")
        .args([
            "attach",
            "-quiet",
            "-readwrite",
            "-noverify",
            "-noautoopen",
            "-nobrowse",
        ])
        .arg("-mountpoint")
        .arg(&volume)
        .arg(&writable))?;
    // 只在 macOS 上执行到这里（其他平台已在前面返回）；条件编译是为了在不支持这些工具的系统上
    // xtask 仍能编译通过。
    #[cfg(unix)]
    let filled = fill_volume(&volume, &app, &background, &icon);
    #[cfg(not(unix))]
    let filled: Result<(), String> = {
        let _ = (&background, &icon);
        Ok(())
    };
    // 填的过程中出错也要先卸载，再报错。
    detach(&volume)?;
    filled?;
    run(Command::new("hdiutil")
        .args(["resize", "-quiet", "-sectors", "min"])
        .arg(&writable))?;

    let image = output.map_or_else(
        || release.join(format!("winc-{version}-{arch}.dmg")),
        Path::to_path_buf,
    );
    if let Some(parent) = image.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let _ = std::fs::remove_file(&image);
    // LZFSE 压缩：macOS 10.11 起支持（winc 要求 11），比 zlib 解压快、压缩率也更高。
    run(Command::new("hdiutil")
        .args(["convert", "-quiet", "-format", "ULFO", "-o"])
        .arg(&image)
        .arg(&writable))?;
    std::fs::remove_dir_all(&work).map_err(io)?;
    run(Command::new("hdiutil")
        .args(["verify", "-quiet"])
        .arg(&image))?;

    let size = std::fs::metadata(&image).map_err(io)?.len() as f64 / 1_048_576.0;
    println!("{}", app.display());
    println!("{}  {size:.2} MB", image.display());
    Ok(())
}

/// 返回应用 `app` 的版本号（`Info.plist` 里的 `CFBundleShortVersionString`）。
///
/// # Errors
///
/// `PlistBuddy` 无法运行、读不到这一项时返回错误描述。
fn bundle_version(app: &Path) -> Result<String, String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleShortVersionString"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .map_err(|e| format!("无法运行 PlistBuddy：{e}"))?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || version.is_empty() {
        return Err(format!("读不到 {} 的版本号", app.display()));
    }
    Ok(version)
}

/// 填充 dmg 的卷 `volume`（挂载着的根目录）：签好名的应用 `app`、「应用程序」的替身、
/// 背景图 `background`、卷图标 `icon`，以及窗口布局（`.DS_Store`）。
///
/// # Errors
///
/// 复制、写文件、`ditto` 等失败时返回原因。
#[cfg(unix)]
fn fill_volume(volume: &Path, app: &Path, background: &Path, icon: &Path) -> Result<(), String> {
    use finder::{Plist, Record, Value};

    let io = |e: std::io::Error| e.to_string();
    // `ditto` 原样保留签名和扩展属性。
    run(Command::new("ditto").arg(app).arg(volume.join("winc.app")))?;
    std::os::unix::fs::symlink("/Applications", volume.join("Applications")).map_err(io)?;
    let hidden = volume.join(".background");
    std::fs::create_dir(&hidden).map_err(io)?;
    let picture = hidden.join("background.tiff");
    std::fs::copy(background, &picture).map_err(io)?;
    // 卷图标用应用图标：挂载后在桌面和侧边栏中显示。
    let volume_icon = volume.join(".VolumeIcon.icns");
    std::fs::copy(icon, &volume_icon).map_err(io)?;
    // 不给 winc.app 设「隐藏扩展名」：该标志存在包根目录的文件信息里，装到「应用程序」以后
    // `codesign --verify --strict` 会把这个标志当成多余的附加数据而判定签名无效。
    set_finder_flags(volume, HAS_CUSTOM_ICON)?;
    // 打开「显示隐藏文件」后仍不显示 `.background` 和 `.VolumeIcon.icns`：两个条目都置上
    // `hidden` 标志。
    run(Command::new("chflags")
        .arg("hidden")
        .arg(&hidden)
        .arg(&volume_icon))?;

    let mount_point = format!("/Volumes/{VOLUME_NAME}");
    let alias = finder::alias(volume, VOLUME_NAME, &mount_point, &picture).map_err(io)?;
    let (x, y) = WINDOW_AT;
    let (w, h) = (WINDOW.0, WINDOW.1 + TITLE_BAR);
    // 窗口只留内容区：没有工具栏、边栏、路径栏、状态栏、标签栏。
    let window = finder::bplist(&[
        ("ContainerShowSidebar", Plist::Bool(false)),
        ("PreviewPaneVisibility", Plist::Bool(false)),
        ("ShowPathbar", Plist::Bool(false)),
        ("ShowSidebar", Plist::Bool(false)),
        ("ShowStatusBar", Plist::Bool(false)),
        ("ShowTabView", Plist::Bool(false)),
        ("ShowToolbar", Plist::Bool(false)),
        ("SidebarWidth", Plist::Int(0)),
        (
            "WindowBounds",
            Plist::Str(format!("{{{{{x}, {y}}}, {{{w}, {h}}}}}")),
        ),
    ]);
    // 图标视图：不自动排列，背景是图片（类型 2），名字在图标下面。
    let view = finder::bplist(&[
        ("arrangeBy", Plist::Str("none".into())),
        ("backgroundColorBlue", Plist::Real(1.0)),
        ("backgroundColorGreen", Plist::Real(1.0)),
        ("backgroundColorRed", Plist::Real(1.0)),
        ("backgroundImageAlias", Plist::Data(alias)),
        ("backgroundType", Plist::Int(2)),
        ("gridOffsetX", Plist::Real(0.0)),
        ("gridOffsetY", Plist::Real(0.0)),
        ("gridSpacing", Plist::Real(100.0)),
        ("iconSize", Plist::Real(ICON_SIZE)),
        ("labelOnBottom", Plist::Bool(true)),
        ("showIconPreview", Plist::Bool(true)),
        ("showItemInfo", Plist::Bool(false)),
        ("textSize", Plist::Real(TEXT_SIZE)),
        ("viewOptionsVersion", Plist::Int(1)),
    ]);
    let root = |code: &[u8; 4], value| Record {
        name: ".".into(),
        code: *code,
        value,
    };
    let store = finder::ds_store(vec![
        root(b"bwsp", Value::Blob(window)),
        root(b"icvp", Value::Blob(view)),
        // 用图标视图打开。
        root(b"vstl", Value::Type(*b"icnv")),
        root(b"vSrn", Value::Long(1)),
        Record::icon_at("winc.app", APP_AT),
        Record::icon_at("Applications", APPLICATIONS_AT),
    ])?;
    std::fs::write(volume.join(".DS_Store"), store).map_err(io)?;

    // 挂载期间系统可能已建了文件系统事件的日志目录，镜像中不需要。
    let _ = std::fs::remove_dir_all(volume.join(".fseventsd"));
    Ok(())
}

/// `FinderInfo` 中的「带自定义图标」标志位（位 10）。
///
/// 置位后卷使用 `.VolumeIcon.icns` 作为图标。
#[cfg(unix)]
const HAS_CUSTOM_ICON: u16 = 0x0400;

/// 把 `path` 的文件信息（扩展属性 `com.apple.FinderInfo`，32 字节）的标志设为 `flags`。
///
/// # Errors
///
/// `xattr` 执行失败时返回原因。
#[cfg(unix)]
fn set_finder_flags(path: &Path, flags: u16) -> Result<(), String> {
    let mut info = [0u8; 32];
    // 文件和目录的标志都在第 8、9 字节。
    info[8..10].copy_from_slice(&flags.to_be_bytes());
    let hex: String = info.iter().map(|b| format!("{b:02x}")).collect();
    run(Command::new("xattr")
        .args(["-wx", "com.apple.FinderInfo", &hex])
        .arg(path))
}

/// 卸载挂在 `volume` 上的镜像。
///
/// 刚写完文件时系统可能还占着卷，等待后重试，最后强制卸载。
///
/// # Errors
///
/// 强制卸载也失败时返回原因。
fn detach(volume: &Path) -> Result<(), String> {
    for _ in 0..5 {
        let status = Command::new("hdiutil")
            .args(["detach", "-quiet"])
            .arg(volume)
            .status()
            .map_err(|e| format!("无法运行 hdiutil：{e}"))?;
        if status.success() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    run(Command::new("hdiutil")
        .args(["detach", "-quiet", "-force"])
        .arg(volume))
}

/// 返回目录 `dir` 下所有文件的总字节数（不跟随符号链接）。
fn dir_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            total += dir_size(&entry.path())?;
        } else if kind.is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

/// 画 dmg 窗口的背景：浅色的竖向渐变，两个图标正中间一个箭头。
///
/// 窗口有背景图时，系统不论外观总用深色文字绘制图标名称，所以背景用浅色。按 1 倍和 2 倍各画一张，
/// 用 `tiffutil` 合成一个 TIFF 写到 `output`，中间文件放在 `work` 下，完成后删除。
///
/// # Errors
///
/// 渲染、写文件、`tiffutil` 失败时返回原因。
fn dmg_background(work: &Path, output: &Path) -> Result<(), String> {
    let (w, h) = (WINDOW.0, BACKGROUND_HEIGHT);
    let fade = WINDOW.1;
    let (left, y) = APP_AT;
    let right = APPLICATIONS_AT.0;
    let x = (left + right) / 2;
    // 箭头长 64 点：杆从 x−32 到 x+32，头是两条 45° 的斜线，各伸出 12 点。
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="{fade}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FCFCFD"/>
      <stop offset="1" stop-color="#ECEEF2"/>
    </linearGradient>
  </defs>
  <rect width="{w}" height="{h}" fill="url(#bg)"/>
  <path d="M {x0} {y} H {x1} M {x2} {y0} L {x1} {y} L {x2} {y1}" fill="none"
        stroke="#9EA5B2" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/>
</svg>"##,
        x0 = x - 32,
        x1 = x + 32,
        x2 = x + 20,
        y0 = y - 12,
        y1 = y + 12,
    );
    let picture = winc_preview::svg::load(&svg, None, None)?;
    let one = work.join("background.png");
    let two = work.join("background@2x.png");
    write_png(&one, &picture.render(w, h))?;
    write_png(&two, &picture.render(w * 2, h * 2))?;
    // `tiffutil` 成功时也会报告写了几张图，只在失败时显示它的输出。
    let tiff = Command::new("tiffutil")
        .arg("-cathidpicheck")
        .arg(&one)
        .arg(&two)
        .arg("-out")
        .arg(output)
        .output()
        .map_err(|e| format!("无法运行 tiffutil：{e}"))?;
    if !tiff.status.success() {
        return Err(format!(
            "tiffutil 失败（{}）：{}",
            tiff.status,
            String::from_utf8_lossy(&tiff.stderr).trim()
        ));
    }
    let _ = std::fs::remove_file(&one);
    let _ = std::fs::remove_file(&two);
    Ok(())
}

/// 应用图标的源文件，相对 workspace 根目录。
const ICON_SOURCE: &str = "crates/winc-app/assets/icon.svg";

/// `.icns` 的文件名（不含扩展名），也是 `Info.plist` 中 `CFBundleIconFile` 的值。
const ICON_NAME: &str = "winc";

/// `iconutil` 要求的图标尺寸：（文件名中的尺寸，倍率）。
///
/// 实际像素为两者之积。
const ICON_SIZES: [(u32, u32); 10] = [
    (16, 1),
    (16, 2),
    (32, 1),
    (32, 2),
    (128, 1),
    (128, 2),
    (256, 1),
    (256, 2),
    (512, 1),
    (512, 2),
];

/// 把 `ICON_SOURCE` 渲染成各个尺寸的 PNG，用 `iconutil` 合成 `.icns` 写到 `output`。
///
/// 每个尺寸都从矢量图直接渲染，不从大图缩小，小尺寸的边缘更清晰。中间文件放在 `work` 下的
/// `.iconset` 目录里，完成后删除。
///
/// # Errors
///
/// 读不到或解析不了 SVG、写文件失败、`iconutil` 失败时返回原因。
fn app_icon(work: &Path, output: &Path) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let source = root().join(ICON_SOURCE);
    let text = std::fs::read_to_string(&source)
        .map_err(|e| format!("读不到 {}：{e}", source.display()))?;
    let picture = winc_preview::svg::load(&text, None, None)?;

    let iconset = work.join(format!("{ICON_NAME}.iconset"));
    let _ = std::fs::remove_dir_all(&iconset);
    std::fs::create_dir_all(&iconset).map_err(io)?;
    for (size, scale) in ICON_SIZES {
        let name = match scale {
            1 => format!("icon_{size}x{size}.png"),
            _ => format!("icon_{size}x{size}@{scale}x.png"),
        };
        let pixels = picture.render(size * scale, size * scale);
        write_png(&iconset.join(name), &pixels)?;
    }

    run(Command::new("iconutil")
        .args(["--convert", "icns", "--output"])
        .arg(output)
        .arg(&iconset))?;
    std::fs::remove_dir_all(&iconset).map_err(io)
}

/// 把 RGBA 像素（未预乘）写成 PNG。
fn write_png(path: &Path, pixels: &winc_preview::image::Pixels) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("{}：{e}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), pixels.width, pixels.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&pixels.rgba))
        .map_err(|e| format!("{}：{e}", path.display()))
}

/// 生成 `Info.plist`；`arch` 是应用的架构（`arm64` 或 `x86_64`），决定请求哪一份 appcast。
///
/// `CFBundleDocumentTypes` 声明能打开任意文件（`public.data`）和文件夹（`public.folder`）：
/// 有了这一项，文件和文件夹才能拖到程序坞的图标上，「打开方式」里也才列出 winc。等级是
/// `Alternate`，不占用其他应用的默认打开方式。
fn info_plist(version: &str, updates: &sparkle::UpdateConfig, arch: &str) -> String {
    let updates = updates.plist_entries(arch);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Document</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Alternate</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.data</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Folder</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Alternate</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.folder</string>
			</array>
		</dict>
	</array>
	<key>CFBundleDevelopmentRegion</key>
	<string>zh_CN</string>
	<key>CFBundleDisplayName</key>
	<string>winc</string>
	<key>CFBundleExecutable</key>
	<string>winc</string>
	<key>CFBundleIconFile</key>
	<string>{ICON_NAME}</string>
	<key>CFBundleIdentifier</key>
	<string>{BUNDLE_ID}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>winc</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.developer-tools</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSPrincipalClass</key>
	<string>NSApplication</string>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
{updates}</dict>
</plist>
"#
    )
}
