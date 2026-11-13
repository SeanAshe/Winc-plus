//! 粘贴文件和图片：复制到目录中时取不重名的名字，粘贴到 Markdown 中时写成图片或链接。

use std::io;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use winc_i18n::t;

use crate::workspace::copying::{Copying, copy_item};

/// 粘贴到 Markdown 时写成图片（`![…](…)`）的文件扩展名，小写；其他文件写成链接。
///
/// 比较前统一转成小写。依据：覆盖常见的位图与矢量图片格式，包括编辑器自己不作预览、写进
/// Markdown 后仍能显示成图片的格式。
const IMAGE_EXTENSIONS: [&str; 13] = [
    "apng", "avif", "bmp", "gif", "heic", "ico", "jpeg", "jpg", "png", "svg", "tif", "tiff", "webp",
];

/// 保存剪贴板中的图片时用的文件名；重名时依次改为 `image-1.png`、`image-2.png` 等。
const IMAGE_NAME: &str = "image.png";

/// 文件 `path` 粘贴到 Markdown 时是否写成图片（按扩展名判断，不区分大小写）。
///
/// 与 [`winc_preview::image::is_image`]（编辑器能否作为图片预览）不同：这里还包括 SVG、AVIF、
/// HEIC、TIFF 等编辑器自己不作预览、但写进 Markdown 后能显示成图片的格式。
fn is_markdown_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// 取不重名文件名的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Naming {
    /// `image.png`、`image-1.png`、`image-2.png`……：粘贴的图片。
    Numbered,
    /// `a.txt`、`a 副本.txt`、`a 副本 2.txt`……：复制来的文件和目录。
    Copy,
}

/// 把文件名分成主名和扩展名（含 `.`）。
///
/// 以 `.` 开头且没有其他 `.` 的名字（如 `.env`、`..`）整个是主名。
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// 返回第 `n` 个候选名字（`n` 为 0 时是原名）。
fn candidate(stem: &str, ext: &str, n: u32, naming: Naming) -> String {
    match (naming, n) {
        (_, 0) => format!("{stem}{ext}"),
        (Naming::Numbered, n) => format!("{stem}-{n}{ext}"),
        (Naming::Copy, 1) => t!("editor-paste-copy-name", stem = stem, ext = ext),
        (Naming::Copy, n) => t!(
            "editor-paste-copy-name-numbered",
            stem = stem,
            number = n,
            ext = ext
        ),
    }
}

/// 返回目录 `dir` 中以 `name` 为基础、不与已有项重名的、可以直接使用的路径。
/// `is_dir` 为真时名字不分扩展名，`naming` 决定重名时怎么取名。
fn free_path(dir: &Path, name: &str, is_dir: bool, naming: Naming) -> PathBuf {
    let (stem, ext) = if is_dir { (name, "") } else { split_name(name) };
    (0u32..)
        .map(|n| dir.join(candidate(stem, ext, n, naming)))
        .find(|path| winc_backend::fs::symlink_metadata(path).is_err())
        .expect("候选名字无穷多，总有一个未被占用")
}

/// 把 PNG 图片 `png` 存到目录 `dir` 中，返回新文件的路径。
///
/// 文件名为 `image.png`，重名时依次加 `-1`、`-2`……，不覆盖已有文件。
///
/// # Errors
///
/// 创建或写入文件失败时返回对应的 `std::io::Error`；写到一半失败时删除写了一半的文件。
pub fn save_image(dir: &Path, png: &[u8]) -> io::Result<PathBuf> {
    loop {
        let path = free_path(dir, IMAGE_NAME, false, Naming::Numbered);
        match winc_backend::fs::create_new(&path, png) {
            Ok(()) => return Ok(path),
            // 取名与创建之间文件名可能已被其他程序占用：此时换下一个名字。
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
}

/// 把文本粘贴为新文件时建议的文件名（不含目录）。
///
/// 按内容猜扩展名，猜不出时为 `.txt`；在目录 `dir` 中不与已有项重名（`untitled.txt`、
/// `untitled-1.txt`……）。
pub fn text_file_name(dir: &Path, text: &str) -> String {
    let name = format!("untitled.{}", guess_extension(text));
    file_name_of(&free_path(dir, &name, false, Naming::Numbered))
}

/// 把图片粘贴为新文件时建议的文件名（不含目录）。
///
/// 文件名为 `image.png`，在目录 `dir` 中重名时依次加 `-1`、`-2` 等。
pub fn image_file_name(dir: &Path) -> String {
    file_name_of(&free_path(dir, IMAGE_NAME, false, Naming::Numbered))
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 按文本开头的内容猜它适合的扩展名（不含 `.`），猜不出时为 `txt`。
///
/// 只看开头和结尾，不解析全文；支持 SVG、HTML、XML、JSON，以及带 `#!` 的脚本。
fn guess_extension(text: &str) -> &'static str {
    let text = text.trim();
    let head: String = text
        .chars()
        .take(256)
        .collect::<String>()
        .to_ascii_lowercase();
    if let Some(shebang) = head.strip_prefix("#!") {
        let line = shebang.lines().next().unwrap_or_default();
        return if line.contains("python") {
            "py"
        } else if line.contains("node") {
            "js"
        } else if ["sh", "bash", "zsh"]
            .iter()
            .any(|shell| line.ends_with(shell))
        {
            "sh"
        } else {
            "txt"
        };
    }
    if head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg")) {
        "svg"
    } else if head.starts_with("<!doctype html") || head.starts_with("<html") {
        "html"
    } else if head.starts_with("<?xml") {
        "xml"
    } else if (text.starts_with('{') && text.ends_with('}'))
        || (text.starts_with('[') && text.ends_with(']'))
    {
        "json"
    } else {
        "txt"
    }
}

/// 把 RGBA 像素（每像素 4 字节，自上而下逐行，共 `width × height` 个）编码为 PNG。
///
/// 用于剪贴板只给出像素的平台；数据长度与尺寸不符或尺寸为 0 时返回 `None`。压缩用 png 库的
/// `Compression::Fast` 一档：截图常有数千万字节的像素，更高的压缩率会让粘贴明显变慢。
pub fn encode_png(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    if w == 0 || h == 0 || rgba.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()?;
    Some(out)
}

/// 复制失败的项及原因。
#[derive(Debug)]
pub struct CopyError {
    /// 复制失败的源路径。
    pub path: PathBuf,
    /// 失败的原因。
    pub error: io::Error,
}

/// 把 `sources` 逐个复制到目录 `dir` 中（目录连同其中的内容），返回新路径，顺序与 `sources` 相同。
///
/// 重名时依次改为 `a 副本.txt`、`a 副本 2.txt`，不覆盖已有项；符号链接复制为指向同一目标的链接。
/// 目录粘贴到自身时在同级位置生成副本。进度记入 `copying`，经它可以取消。
///
/// # Errors
///
/// `sources` 中的项复制失败时停止，返回失败的那一项和原因，先前复制好的项保留；复制到自身
/// 之内时返回 `InvalidInput`，被取消时返回 `Interrupted`（正在复制的一项已复制出的部分随之
/// 删除）。
pub fn copy_into(
    sources: &[PathBuf],
    dir: &Path,
    copying: &Copying,
) -> Result<Vec<PathBuf>, CopyError> {
    sources
        .iter()
        .map(|source| {
            let dir = match source.parent() {
                Some(parent) if same(source, dir) => parent,
                _ => dir,
            };
            copy_one(source, dir, Naming::Copy, copying).map_err(|error| CopyError {
                path: source.clone(),
                error,
            })
        })
        .collect()
}

/// 把 `source` 复制到目录 `dir` 中，重名时按 `naming` 取名，返回新路径。
fn copy_one(source: &Path, dir: &Path, naming: Naming, copying: &Copying) -> io::Result<PathBuf> {
    let meta = winc_backend::fs::symlink_metadata(source)?;
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, t!("editor-paste-cannot-copy")))?
        .to_string_lossy();

    if meta.is_dir() && inside(dir, source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            t!("editor-paste-into-itself"),
        ));
    }

    let target = free_path(dir, &name, meta.is_dir(), naming);
    copy_item(source, &target, copy_symlink, copying)?;
    Ok(target)
}

/// 是否 `a` 和 `b` 指同一处（两边都能解析成真实路径时按真实路径比较）。
fn same(a: &Path, b: &Path) -> bool {
    match (
        winc_config::files::canonicalize(a),
        winc_config::files::canonicalize(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 是否 `path` 就是 `dir` 或在 `dir` 之内。
///
/// 两边都能解析成真实路径时按真实路径比较（`/tmp` 与 `/private/tmp` 视为同一处），否则按
/// 字面比较。
fn inside(path: &Path, dir: &Path) -> bool {
    match (
        winc_config::files::canonicalize(path),
        winc_config::files::canonicalize(dir),
    ) {
        (Ok(path), Ok(dir)) => path.starts_with(dir),
        _ => path.starts_with(dir),
    }
}

/// 把 `from` 复制为 `to`：目录逐项递归复制，符号链接复制链接本身。
pub(crate) fn copy_entry(from: &Path, to: &Path) -> io::Result<()> {
    copy_item(from, to, copy_symlink, &Copying::default())
}

/// 把符号链接 `from` 复制为指向同一目标的符号链接 `to`。
#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> io::Result<()> {
    winc_backend::fs::copy_symlink(from, to)
}

/// 其他平台上创建符号链接需要额外权限，复制链接指向的内容。
#[cfg(not(unix))]
fn copy_symlink(from: &Path, to: &Path) -> io::Result<()> {
    winc_backend::fs::copy(from, to).map(drop)
}

/// 准备粘贴到 Markdown 文档中的文件，返回要写进链接的路径，顺序与 `files` 相同。
///
/// 位于项目目录 `root`（没有项目时为文档所在的目录 `doc_dir`）之外的图片先复制到 `doc_dir`
/// 中，重名时加 `-1`、`-2`……；其余文件原样引用。
///
/// # Errors
///
/// 复制某张图片失败时停止，返回失败的那一项和原因，先前复制好的图片保留。
pub fn bring_images(
    doc_dir: &Path,
    root: Option<&Path>,
    files: &[PathBuf],
) -> Result<Vec<PathBuf>, CopyError> {
    let home = root.unwrap_or(doc_dir);
    files
        .iter()
        .map(|file| {
            if !is_markdown_image(file) || inside(file, home) {
                return Ok(file.clone());
            }
            copy_one(file, doc_dir, Naming::Numbered, &Copying::default()).map_err(|error| {
                CopyError {
                    path: file.clone(),
                    error,
                }
            })
        })
        .collect()
}

/// 粘贴到 Markdown 文档中的一段文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// 要插入的文本。
    pub text: String,
    /// 只有一项时，其中占位文字（图片的替代文字或链接文字）在 `text` 中的字符范围（`char` 偏移，
    /// 左闭右开）；粘贴后选中，直接键入即可替换。
    ///
    /// 多项时为 `None`。
    pub placeholder: Option<Range<usize>>,
}

/// 把文件 `files` 写成 Markdown 链接，多项之间用一个空格隔开，返回插入的文本。
///
/// 图片写成 `![主名](路径)`，其他文件写成 `[文件名](路径)`。位于项目目录 `root` 或文档所在的
/// 目录 `doc_dir` 之内的文件写成相对 `doc_dir` 的路径，其余（以及文档尚未保存时）写成绝对路径；
/// 含空白、控制字符或括号不配对时用 `<…>` 括起来。
pub fn markdown_links(doc_dir: Option<&Path>, root: Option<&Path>, files: &[PathBuf]) -> Snippet {
    let mut text = String::new();
    let mut placeholder = None;
    for (i, file) in files.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        let image = is_markdown_image(file);
        let label = if image {
            file.file_stem()
        } else {
            file.file_name()
        };
        let label = escape_brackets(&label.unwrap_or_default().to_string_lossy());
        let opening = if image { "![" } else { "[" };

        text.push_str(opening);
        let start = text.chars().count();
        text.push_str(&label);
        placeholder = Some(start..start + label.chars().count());
        text.push_str("](");
        text.push_str(&escape_link_path(&link_path(doc_dir, root, file)));
        text.push(')');
    }
    Snippet {
        text,
        placeholder: placeholder.filter(|_| files.len() == 1),
    }
}

/// 返回链接中写的路径：在 `root` 或 `doc_dir` 之内时相对 `doc_dir`，否则为绝对路径。
fn link_path(doc_dir: Option<&Path>, root: Option<&Path>, file: &Path) -> String {
    let relative = doc_dir
        .filter(|dir| inside(file, dir) || root.is_some_and(|root| inside(file, root)))
        .and_then(|dir| relative(dir, file));
    let mut out = String::new();
    for part in relative.as_deref().unwrap_or(file).components() {
        match part {
            // 盘符（`C:`）原样写出，根目录写成 `/`，其余各部分之间用 `/` 隔开。
            Component::Prefix(prefix) => out.push_str(&prefix.as_os_str().to_string_lossy()),
            Component::RootDir => out.push('/'),
            part => {
                if !out.is_empty() && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(&part.as_os_str().to_string_lossy());
            }
        }
    }
    out
}

/// 返回从目录 `from` 到 `to` 的相对路径；两者没有共同的上级（例如在不同的盘上）时返回 `None`。
fn relative(from: &Path, to: &Path) -> Option<PathBuf> {
    let (from, to) = match (
        winc_config::files::canonicalize(from),
        winc_config::files::canonicalize(to),
    ) {
        (Ok(from), Ok(to)) => (from, to),
        _ => (from.to_path_buf(), to.to_path_buf()),
    };
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return None;
    }
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for part in &to[common..] {
        out.push(part);
    }
    Some(out)
}

/// 转义链接文字中的方括号，避免提前结束链接文字。
fn escape_brackets(text: &str) -> String {
    text.replace('[', "\\[").replace(']', "\\]")
}

/// 按 `CommonMark` 的规则写出链接目标：含空白、控制字符、以 `<` 开头或括号不配对时用 `<…>`
/// 括起来（其中的 `<`、`>` 加反斜杠），否则原样写出。
fn escape_link_path(path: &str) -> String {
    if needs_brackets(path) {
        format!("<{}>", path.replace('<', "\\<").replace('>', "\\>"))
    } else {
        path.to_owned()
    }
}

/// 链接目标是否需要用 `<…>` 括起来。
fn needs_brackets(path: &str) -> bool {
    if path.starts_with('<') || path.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return true;
    }
    // 不带尖括号的链接目标中，括号必须配对（转义的除外），否则链接在不配对的 `)` 处结束。
    let mut depth = 0i32;
    let mut escaped = false;
    for c in path.chars() {
        match c {
            '(' if !escaped => depth += 1,
            ')' if !escaped => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            _ => {}
        }
        escaped = c == '\\' && !escaped;
    }
    depth != 0
}

#[cfg(test)]
mod tests;
