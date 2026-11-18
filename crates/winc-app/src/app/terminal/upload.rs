//! 粘贴到远端终端的图片和文件：先传到那台主机上，再把那里的路径粘贴给程序。
//!
//! 终端里的程序（Claude Code、Codex、Pi 等）粘贴图片时读的是程序所在机器的剪贴板；程序在远端
//! 主机上运行时，本机剪贴板里的截图和从文件管理器复制的文件它都读不到。这些程序都接受粘贴进来
//! 的图片路径（与把文件拖进终端相同），因此把剪贴板里的图片或本机文件传到那台主机上的缓存目录
//! （[`PREPARE`]），再把那里的路径作为一次粘贴发给程序。
//!
//! 传到的是标签所在的那台主机：在标签里又 `ssh` 到别的主机时，那里的程序打不开这个路径。

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use winc_backend::Target;
use winc_backend::remote::{self, Exec};
use winc_i18n::t;

use super::editor::{shell_quote, shell_safe};
use super::*;
use crate::app::editor::clipboard::ClipImage;

/// 在远端准备存放目录的脚本：`$XDG_CACHE_HOME/winc/paste`（没有设置时为 `~/.cache/winc/paste`），
/// 只有用户本人可以访问（截图里可能有不想给同一台主机上其他用户看的内容）。顺便删掉 7 天前传上来
/// 的文件，目录不会越积越大。输出目录的绝对路径。
pub(super) const PREPARE: &str = concat!(
    r#"d="${XDG_CACHE_HOME:-$HOME/.cache}/winc/paste"; "#,
    r#"mkdir -p "$d" && chmod 700 "$d" || exit 1; "#,
    r#"find "$d" -type f -mtime +7 -exec rm -f {} + 2>/dev/null; "#,
    r#"printf %s "$d""#,
);

/// 准备存放目录最长等待的时间（10 秒）：连接没有响应时不一直等下去。
const PREPARE_TIMEOUT: Duration = Duration::from_secs(10);

/// 一次粘贴最多传的大小（字节）：截图和随手附上的文件远小于此，误粘贴的大文件不会在后台一直
/// 传下去。取自调参。
pub(crate) const MAX_UPLOAD: u64 = 50 << 20;

/// 要传到远端的剪贴板内容。
pub(crate) enum Payload {
    /// 剪贴板里的图片，在后台线程上编码。
    Image(ClipImage),
    /// 从文件管理器复制的文件（不含目录，总大小不超过 [`MAX_UPLOAD`]）。
    Files(Vec<PathBuf>),
}

/// 测试中代替远端 `$XDG_CACHE_HOME` 的目录，避免写进运行测试的用户的主目录。
#[cfg(test)]
pub(crate) static CACHE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// 把 `payload` 传到 `target` 上的存放目录，返回各文件在那台主机上的路径。
///
/// 等待远端的应答，读文件、编码图片也在这里：在后台线程上调用。名字是 `<毫秒时间戳>-<名字>`
/// （图片的名字是 `paste.png`），重名时在时间戳后加编号；文件名里 shell 需要引号的字符换成
/// `_`，粘贴后的路径不需要引号，各个程序都能认出来。
///
/// # Errors
///
/// 连接已断开、目录无法创建、读不了本机文件、图片超过 [`MAX_UPLOAD`]、写入失败时返回对应的
/// 错误。
pub(crate) fn upload(target: &Target, payload: Payload) -> io::Result<Vec<String>> {
    let items: Vec<(String, Vec<u8>)> = match payload {
        Payload::Image(image) => {
            let png = image
                .into_png()
                .ok_or_else(|| io::Error::other(t!("terminal-upload-no-image")))?;
            if png.len() as u64 > MAX_UPLOAD {
                return Err(io::Error::other(too_large(png.len() as u64)));
            }
            vec![("paste.png".to_owned(), png)]
        }
        Payload::Files(files) => (files.iter())
            .map(|file| {
                let name = file.file_name().map(|n| n.to_string_lossy());
                let name = safe_name(name.as_deref().unwrap_or("file"));
                Ok((name, winc_backend::fs::read(file)?))
            })
            .collect::<io::Result<_>>()?,
    };
    let dir = prepare(target)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    let mut uploaded = Vec::new();
    for (name, bytes) in items {
        uploaded.push(write_new(&dir, stamp, &name, &bytes)?);
    }
    Ok((uploaded.iter())
        .map(|path| winc_backend::native_lossy(path).into_owned())
        .collect())
}

/// 说明 `size` 字节超过了 [`MAX_UPLOAD`]、没有传的提示。
fn too_large(size: u64) -> String {
    use winc_preview::image::size_label;
    t!(
        "terminal-upload-too-large",
        size = size_label(size),
        limit = size_label(MAX_UPLOAD)
    )
}

/// 在 `dir` 中新建 `<stamp>-<name>` 并写入 `bytes`，返回它的路径；已有同名文件时在时间戳后加
/// 编号再试，不覆盖已有的文件。
fn write_new(dir: &Path, stamp: u128, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let mut n = 0;
    loop {
        let file = match n {
            0 => format!("{stamp}-{name}"),
            n => format!("{stamp}-{n}-{name}"),
        };
        let path = winc_backend::join(dir, &file);
        match winc_backend::fs::create_new(&path, bytes) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < 100 => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// 在 `target` 上准备存放目录（[`PREPARE`]），返回它的虚拟路径。
fn prepare(target: &Target) -> io::Result<PathBuf> {
    #[allow(unused_mut, reason = "测试中另设缓存目录")]
    let mut exec = Exec::new("sh").arg("-c").arg(PREPARE);
    #[cfg(test)]
    if let Some(cache) = CACHE.lock().unwrap().clone() {
        exec = exec.env("XDG_CACHE_HOME", os_bytes(cache.as_os_str()));
    }
    let root = winc_backend::virtual_path(target, b"/");
    let deadline = Instant::now() + PREPARE_TIMEOUT;
    let output = remote::exec(&root, &exec, &|| Instant::now() >= deadline)?;
    if output.code != Some(0) || !output.stdout.starts_with(b"/") {
        let reason = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let reason = if reason.is_empty() {
            t!("terminal-upload-no-folder").to_owned()
        } else {
            reason
        };
        return Err(io::Error::other(reason));
    }
    Ok(winc_backend::virtual_path(target, &output.stdout))
}

/// 把文件名中 shell 需要引号的字符（见 [`shell_safe`]）换成 `_`；`/` 也换掉。
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| if shell_safe(c) && c != '/' { c } else { '_' })
        .collect()
}

/// 把传好的路径写成一次粘贴的文字：以空格隔开，需要时加引号（主目录里有空格时）。
pub(super) fn paste_text(paths: &[String]) -> String {
    let quoted: Vec<String> = (paths.iter())
        .map(|path| shell_quote(Path::new(path)))
        .collect();
    quoted.join(" ")
}

impl App {
    /// 正在看的标签在远端主机上时，把剪贴板里的本机文件，或者（`text_empty` 为真，剪贴板里
    /// 没有文字时）图片传到那台主机，传好后把那里的路径粘贴给程序（[`App::terminal_uploaded`]）。
    /// 返回是否这样处理了；标签在本机、或剪贴板里没有可传的内容时返回 `false`，按原来的方式粘贴。
    ///
    /// 文件先于文字：从文件管理器复制文件时剪贴板里也有文件名的文字。复制的是目录、或文件总共
    /// 超过 [`MAX_UPLOAD`] 时说明原因，不传。传输在后台进行；测试中没有事件循环，就地完成。
    pub(super) fn upload_paste(&mut self, id: u16, cwd: &[u8], text_empty: bool) -> bool {
        let Some(target) = winc_backend::locate(&bytes_path(cwd)).map(|at| at.target) else {
            return false;
        };
        let files = self.clipboard.files();
        let payload = if !files.is_empty() {
            if let Some(folder) = files.iter().find(|file| winc_backend::fs::is_dir(file)) {
                let name = winc_editor::display_name(folder);
                self.notify(t!("terminal-upload-folder", name = name));
                return true;
            }
            let size = (files.iter())
                .filter_map(|file| winc_backend::fs::metadata(file).ok())
                .map(|meta| meta.len)
                .sum::<u64>();
            if size > MAX_UPLOAD {
                self.notify(too_large(size));
                return true;
            }
            Payload::Files(files)
        } else if let Some(image) = self.clipboard.image().filter(|_| text_empty) {
            Payload::Image(image)
        } else {
            return false;
        };
        let host = target.as_str().to_owned();
        let task = move || upload(&target, payload).map_err(|e| e.to_string());
        match self.link.proxy.clone() {
            Some(proxy) => {
                self.flash(t!("terminal-upload-started", host = host));
                winc_editor::background::spawn_blocking(move || {
                    let _ = proxy.send_event(UserEvent::TerminalUploaded(id, task()));
                });
            }
            None => {
                let result = winc_backend::remote::allow_blocking(task);
                self.terminal_uploaded(id, result);
            }
        }
        true
    }

    /// 剪贴板里的内容已传到远端（[`App::upload_paste`]）：把那里的路径粘贴到标签 `id`；该标签
    /// 已不是正在看的标签时不粘贴，提示传到了哪里。失败时说明原因。
    pub(crate) fn terminal_uploaded(&mut self, id: u16, result: Result<Vec<String>, String>) {
        let paths = match result {
            Ok(paths) => paths,
            Err(error) => {
                self.notify(t!("terminal-upload-failed", error = error));
                return;
            }
        };
        let text = paste_text(&paths);
        if self.terminal.tab().is_some_and(|tab| tab.id == id) {
            self.confirmed_terminal_paste(id, text);
        } else {
            self.notify(t!("terminal-upload-elsewhere", path = text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploaded_names_need_no_quotes() {
        assert_eq!(
            safe_name("Screenshot 2026-11-18 at 10.png"),
            "Screenshot_2026-11-18_at_10.png"
        );
        assert_eq!(safe_name("a'b\"c$(d).txt"), "a_b_c__d_.txt");
        assert_eq!(safe_name("截图 1.png"), "截图_1.png");
        assert_eq!(
            paste_text(&["/home/me/.cache/winc/paste/paste-1.png".into()]),
            "/home/me/.cache/winc/paste/paste-1.png"
        );
        assert_eq!(
            paste_text(&["/home/a b/x.png".into(), "/tmp/y.png".into()]),
            "'/home/a b/x.png' /tmp/y.png"
        );
    }

    /// 准备目录的脚本：目录建在 `$XDG_CACHE_HOME` 下，只有本人可以访问，旧文件被删掉。
    #[cfg(unix)]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "测试只访问本机的临时目录")]
    fn the_paste_folder_is_private_and_old_files_are_removed() {
        use std::os::unix::fs::PermissionsExt;
        {
            let cache = std::env::temp_dir().join(format!("winc-paste-dir-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&cache);
            let dir = cache.join("winc/paste");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("old.png"), "x").unwrap();
            std::fs::write(dir.join("new.png"), "x").unwrap();
            let touched = std::process::Command::new("touch")
                .args(["-t", "202001010000"])
                .arg(dir.join("old.png"))
                .status()
                .unwrap();
            assert!(touched.success());
            let output = std::process::Command::new("sh")
                .args(["-c", PREPARE])
                .env("XDG_CACHE_HOME", &cache)
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(Path::new(std::str::from_utf8(&output.stdout).unwrap()), dir);
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
            assert!(!dir.join("old.png").exists() && dir.join("new.png").exists());
            std::fs::remove_dir_all(&cache).unwrap();
        }
    }
}
