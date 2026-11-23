//! 用本机的 TeX 编译 LaTeX 文档：找程序，判断主文件和引擎，运行编译并读出错误和进度。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::tools::shell::Shell;
use crate::tools::shell::process::{self, Cancel, RunError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 在文件开头多少行内找 `% !TEX` 指令。
pub const MAGIC_LINES: usize = 20;

/// 排版引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Engine {
    /// `pdflatex`。
    Pdf,
    /// `xelatex`。
    Xe,
    /// `lualatex`。
    Lua,
}

impl Engine {
    /// 引擎的程序名。
    pub fn program(self) -> &'static str {
        match self {
            Engine::Pdf => "pdflatex",
            Engine::Xe => "xelatex",
            Engine::Lua => "lualatex",
        }
    }

    /// 由程序名（`xelatex`、`XeLaTeX`）认出引擎。
    fn named(name: &str) -> Option<Engine> {
        match name.trim().to_ascii_lowercase().as_str() {
            "pdflatex" | "latex" | "pdftex" => Some(Engine::Pdf),
            "xelatex" | "xetex" => Some(Engine::Xe),
            "lualatex" | "luatex" | "lualatexmk" => Some(Engine::Lua),
            _ => None,
        }
    }
}

/// 找到的 TeX 程序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Toolchain {
    /// `latexmk`。
    Latexmk(PathBuf),
    /// `tectonic`。
    Tectonic(PathBuf),
    /// 只有引擎，按 [`Engine`] 的顺序。
    Engines(Vec<(Engine, PathBuf)>),
}

/// 找不到 TeX 程序的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// 没有装 TeX。
    NotInstalled,
    /// 设置的程序不存在。
    NotFound(String),
}

/// 找本机的 TeX 程序：`program` 是设置的程序名或路径，为空时自动查找。
///
/// # Errors
///
/// 找不到时返回 [`Missing`]。
pub fn find(shell: &Shell, program: Option<&str>) -> Result<Toolchain, Missing> {
    if let Some(text) = program.map(str::trim).filter(|t| !t.is_empty()) {
        let path = locate(shell, text).ok_or_else(|| Missing::NotFound(text.to_owned()))?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase());
        let stem = stem.unwrap_or_default();
        return Ok(if stem.contains("tectonic") {
            Toolchain::Tectonic(path)
        } else if let Some(engine) = Engine::named(&stem) {
            Toolchain::Engines(vec![(engine, path)])
        } else {
            Toolchain::Latexmk(path)
        });
    }
    if let Some(path) = shell.find("latexmk") {
        return Ok(Toolchain::Latexmk(path));
    }
    if let Some(path) = shell.find("tectonic") {
        return Ok(Toolchain::Tectonic(path));
    }
    let engines: Vec<_> = [Engine::Pdf, Engine::Xe, Engine::Lua]
        .into_iter()
        .filter_map(|e| shell.find(e.program()).map(|path| (e, path)))
        .collect();
    if engines.is_empty() {
        return Err(Missing::NotInstalled);
    }
    Ok(Toolchain::Engines(engines))
}

/// 查找设置的程序 `text`：路径（`~/` 指主目录）或 `PATH` 里的程序名。
fn locate(shell: &Shell, text: &str) -> Option<PathBuf> {
    if !text.contains(['/', '\\']) {
        return shell.find(text);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let path = match (text.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(text),
    };
    winc_backend::fs::is_file(&path).then_some(path)
}

/// 文件 `path` 是否能在本机编译。
pub fn compiles_here(path: &Path) -> bool {
    !winc_backend::is_remote(path) && crate::wsl::locate_here(path).is_none()
}

/// 返回文件开头 [`MAGIC_LINES`] 行内 `% !TEX key = 值` 的值。
pub fn magic<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().take(MAGIC_LINES).find_map(|line| {
        let rest = line.trim_start().strip_prefix('%')?.trim_start();
        let rest = rest
            .get(..4)
            .filter(|t| t.eq_ignore_ascii_case("!tex"))
            .map(|_| &rest[4..])?;
        let (name, value) = rest.split_once('=')?;
        let name = name.trim().to_ascii_lowercase();
        let name = name.strip_prefix("ts-").unwrap_or(&name);
        (name == key.to_ascii_lowercase())
            .then(|| value.trim())
            .filter(|v| !v.is_empty())
    })
}

/// 返回去掉 `%` 注释后的原文。
fn uncommented(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let bytes = line.as_bytes();
        let cut = (0..bytes.len())
            .find(|&i| {
                bytes[i] == b'%'
                    && bytes[..i].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 0
            })
            .unwrap_or(bytes.len());
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

/// 返回 `text` 中命令 `\name` 的各个 `{…}` 参数。
fn arguments<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let needle = format!("\\{name}");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(&needle).map(|i| from + i) {
        let mut rest = &text[at + needle.len()..];
        from = at + needle.len();
        // `\inputenc` 不是 `\input`。
        if rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        rest = rest.trim_start();
        while let Some(opt) = rest.strip_prefix('[') {
            rest = opt.split_once(']').map_or("", |(_, r)| r).trim_start();
        }
        if let Some(arg) = rest.strip_prefix('{').and_then(|r| r.split_once('}')) {
            out.push(arg.0);
        }
    }
    out
}

/// 原文是不是一篇完整的文档（有 `\documentclass`）。
fn is_document(text: &str) -> bool {
    !arguments(&uncommented(text), "documentclass").is_empty()
}

/// 返回编译 `file` 时的主文件：`% !TEX root`、`file` 自己，或 `candidates` 中引入它的文档。
pub fn root_of(file: &Path, text: &str, candidates: &[(PathBuf, String)]) -> Option<PathBuf> {
    let dir = file.parent().unwrap_or(Path::new(""));
    if let Some(root) = magic(text, "root") {
        return Some(normalize(&dir.join(root)));
    }
    if is_document(text) {
        return Some(file.to_path_buf());
    }
    let target = normalize(file);
    candidates
        .iter()
        .filter(|(path, text)| *path != target && is_document(text))
        .find(|(path, text)| {
            let base = path.parent().unwrap_or(Path::new(""));
            let body = uncommented(text);
            ["input", "include", "subfile"]
                .iter()
                .flat_map(|name| arguments(&body, name))
                .any(|arg| {
                    let arg = Path::new(arg.trim());
                    let with_ext = match arg.extension() {
                        Some(_) => arg.to_path_buf(),
                        None => arg.with_extension("tex"),
                    };
                    normalize(&base.join(with_ext)) == target
                })
        })
        .map(|(path, _)| path.clone())
}

/// 去掉路径里的 `.` 和 `..`（不访问文件系统）。
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// 返回主文件原文 `text` 要用的引擎。
pub fn engine_of(text: &str) -> Engine {
    if let Some(engine) = magic(text, "program").and_then(Engine::named) {
        return engine;
    }
    let body = uncommented(text);
    let class = arguments(&body, "documentclass");
    let packages = ["usepackage", "RequirePackage"]
        .iter()
        .flat_map(|name| arguments(&body, name))
        .flat_map(|list| list.split(','))
        .map(str::trim);
    let xe = |name: &str| {
        matches!(
            name,
            "ctex" | "xeCJK" | "fontspec" | "unicode-math" | "xltxtra"
        ) || name.starts_with("ctex")
    };
    if class.iter().map(|c| c.trim()).chain(packages).any(xe) {
        Engine::Xe
    } else {
        Engine::Pdf
    }
}

/// 一次编译：运行的程序、参数、工作目录和产物的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// 编译的主文件。
    pub root: PathBuf,
    /// 运行的程序。
    pub program: PathBuf,
    /// 参数，最后一个是主文件名。
    pub args: Vec<OsString>,
    /// 工作目录：主文件所在的目录。
    pub dir: PathBuf,
    /// 编译出的 PDF。
    pub pdf: PathBuf,
    /// 源码与 PDF 位置的对照（`.synctex.gz`）。
    pub synctex: PathBuf,
    /// 编译日志。
    pub log: PathBuf,
}

/// 主文件所在目录中的 `latexmkrc`。
///
/// 该文件是一段 Perl 程序，`latexmk` 启动时执行；它来自项目，未经用户同意不执行。
#[derive(Debug, Clone, Copy)]
pub struct ProjectRc<'a> {
    /// 文件的内容。
    pub text: &'a str,
    /// 用户是否同意过执行这份内容。
    pub run: bool,
}

/// 返回 `latexmkrc` 的内容 `rc` 中 `$pdf_mode` 指定的引擎；没有设置或是其他方式时返回 `None`。
///
/// `latexmk` 的取值：1 为 pdflatex，4 为 lualatex，5 为 xelatex。
fn pdf_mode(rc: &str) -> Option<Engine> {
    let (_, value) = rc.split_once("$pdf_mode")?;
    let value = value.trim_start().strip_prefix('=')?.trim_start();
    match value.chars().next()? {
        '1' => Some(Engine::Pdf),
        '4' => Some(Engine::Lua),
        '5' => Some(Engine::Xe),
        _ => None,
    }
}

/// 系统级 `latexmkrc` 所在的目录，顺序与 `latexmk` 查找的相同。
const SYSTEM_RC_DIRS: &[&str] = if cfg!(windows) {
    &["C:/latexmk"]
} else {
    &[
        "/etc",
        "/opt/local/share/latexmk",
        "/usr/local/share/latexmk",
        "/usr/local/lib/latexmk",
    ]
};

/// 返回系统级和用户自己的 `latexmkrc`，按 `latexmk` 读取的顺序；两级各取存在的第一个。
///
/// 不读取项目的配置文件时（`-norc`）这两级也不再读取，因此逐个以 `-r` 传给 `latexmk`。
/// 环境变量 `LATEXMKRCSYS` 指定了系统级的文件时只用它。
#[allow(clippy::disallowed_methods, reason = "只访问本机：编译只在本机进行")]
fn own_rc_files() -> Vec<PathBuf> {
    /// 返回 `files` 中存在的第一个。
    fn first(files: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
        files.into_iter().find(|file| file.is_file())
    }
    let var = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty());
    let var = |key: &str| var(key).map(PathBuf::from);
    let system =
        match var("LATEXMKRCSYS") {
            Some(file) => first([file]),
            None => first(["LatexMk", "latexmkrc"].iter().flat_map(|name| {
                (SYSTEM_RC_DIRS.iter()).map(move |dir| Path::new(dir).join(name))
            })),
        };
    let home = var("HOME").or_else(|| var("USERPROFILE"));
    let config = var("XDG_CONFIG_HOME").or_else(|| home.as_ref().map(|home| home.join(".config")));
    let user = [
        config.map(|config| config.join("latexmk/latexmkrc")),
        home.map(|home| home.join(".latexmkrc")),
    ];
    system
        .into_iter()
        .chain(first(user.into_iter().flatten()))
        .collect()
}

/// 返回编译 `root` 的命令；`rc` 是主文件所在目录中的 `latexmkrc`，没有时为 `None`。
///
/// 用户同意过的 `latexmkrc` 由 `latexmk` 读取，其中设了 `$pdf_mode` 时不指定引擎。没有同意的
/// 不读取（`-norc`），只沿用其中 `$pdf_mode` 选的引擎，系统级和用户自己的配置文件另行传入。
pub fn job(
    tool: &Toolchain,
    root: &Path,
    engine: Engine,
    out: &Path,
    rc: Option<ProjectRc<'_>>,
) -> Job {
    let dir = root.parent().unwrap_or(Path::new("")).to_path_buf();
    // 写成 `./名字`：以 `-` 开头的文件名不会被当作选项。
    // 各平台都用 `/`，TeX 把 `\` 当作命令的开头。
    let name = root.file_name().map(|name| {
        let mut path = OsString::from("./");
        path.push(name);
        path
    });
    let name = name.unwrap_or_default();
    let stem = root
        .file_stem()
        .map_or_else(|| "main".into(), |s| s.to_string_lossy().into_owned());
    let mut args: Vec<OsString> = Vec::new();
    let program = match tool {
        Toolchain::Latexmk(path) => {
            let chosen = rc.and_then(|rc| pdf_mode(rc.text));
            let reads_rc = rc.is_none_or(|rc| rc.run);
            if !reads_rc {
                args.push("-norc".into());
                for file in own_rc_files() {
                    args.extend(["-r".into(), file.into_os_string()]);
                }
            }
            if !(reads_rc && chosen.is_some()) {
                args.push(
                    match chosen.unwrap_or(engine) {
                        Engine::Pdf => "-pdf",
                        Engine::Xe => "-xelatex",
                        Engine::Lua => "-lualatex",
                    }
                    .into(),
                );
            }
            args.extend(
                ["-synctex=1", "-interaction=nonstopmode", "-file-line-error"].map(OsString::from),
            );
            args.push(flag("-outdir=", out));
            path.clone()
        }
        Toolchain::Tectonic(path) => {
            // 出错后继续排版并写出 PDF。
            args.extend(
                [
                    "-Z",
                    "continue-on-errors",
                    "--synctex",
                    "--keep-logs",
                    "--outdir",
                ]
                .map(OsString::from),
            );
            args.push(out.as_os_str().to_owned());
            path.clone()
        }
        Toolchain::Engines(engines) => {
            // 没有要用的引擎时用装了的第一个。
            let path = engines
                .iter()
                .find(|(e, _)| *e == engine)
                .or_else(|| engines.first())
                .map(|(_, path)| path.clone())
                .unwrap_or_default();
            args.extend(
                ["-synctex=1", "-interaction=nonstopmode", "-file-line-error"].map(OsString::from),
            );
            args.push(flag("-output-directory=", out));
            path
        }
    };
    args.push(name);
    Job {
        root: root.to_path_buf(),
        program,
        args,
        dir,
        pdf: out.join(format!("{stem}.pdf")),
        synctex: out.join(format!("{stem}.synctex.gz")),
        log: out.join(format!("{stem}.log")),
    }
}

/// `-outdir=路径` 这样连在一起的参数。
fn flag(name: &str, path: &Path) -> OsString {
    let mut arg = OsString::from(name);
    arg.push(path.as_os_str());
    arg
}

/// 返回主文件 `root` 在缓存目录 `cache` 下的产物目录。
pub fn out_dir(cache: &Path, root: &Path) -> PathBuf {
    // FNV-1a，跨版本稳定。
    let hash = root
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    let stem = root
        .file_stem()
        .map_or_else(|| "main".into(), |s| s.to_string_lossy().into_owned());
    cache.join("latex").join(format!("{stem}-{hash:016x}"))
}

/// 编译没有任何输出多久后停下。
const STALL: std::time::Duration = if cfg!(test) {
    // 测试里等不了五分钟。
    std::time::Duration::from_secs(1)
} else {
    std::time::Duration::from_secs(300)
};
/// 一次编译最长运行的时间。
const COMPILE_LIMIT: std::time::Duration = std::time::Duration::from_secs(3600);

/// 日志里最多读出的错误条数。
const MAX_PROBLEMS: usize = 100;

/// 编译日志中的一条错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// 出错的文件。
    pub file: Option<PathBuf>,
    /// 出错的行（从 1 起）。
    pub line: Option<usize>,
    /// 错误信息。
    pub message: String,
}

/// 一次编译的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// 是否有可以显示的 PDF。
    pub pdf: bool,
    /// 日志中的错误。
    pub problems: Vec<Problem>,
    /// 编译没能完成又读不出错误时的原因。
    pub failure: Option<String>,
}

/// 编译进行到哪一步，从程序的输出读出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// 正在下载宏包文件。
    Download(String),
    /// 第几遍排版。
    Pass(u32),
    /// 排到第几页。
    Page(u32),
}

/// 从编译程序的输出中读进度。
#[derive(Debug, Default)]
struct Reader {
    /// 还没有换行的半行。
    pending: String,
    pass: u32,
}

impl Reader {
    /// 读一段输出，返回其中最后一处进度。
    fn feed(&mut self, chunk: &str) -> Option<Progress> {
        let mut last = None;
        self.pending.push_str(chunk);
        while let Some(end) = self.pending.find(['\n', '\r']) {
            let line: String = self.pending.drain(..=end).collect();
            last = self.line(line.trim()).or(last);
        }
        // 页号继续印在同一行里，不等换行。
        let page = |text: &str| {
            let at = text.rfind('[')?;
            let digits: String = text[at + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok().map(Progress::Page)
        };
        page(chunk).or(last)
    }

    fn line(&mut self, line: &str) -> Option<Progress> {
        if let Some(name) = line.strip_prefix("note: downloading ") {
            return Some(Progress::Download(name.trim().to_owned()));
        }
        if let Some(rest) = line.strip_prefix("Run number ") {
            self.pass = rest.split_whitespace().next()?.parse().ok()?;
            return Some(Progress::Pass(self.pass));
        }
        if line.starts_with("note: Rerunning TeX") {
            self.pass = self.pass.max(1) + 1;
            return Some(Progress::Pass(self.pass));
        }
        None
    }
}

/// 运行编译 `job` 并读出错误，进度交给 `progress`；被 `cancel` 取消时返回 `None`。
#[allow(
    clippy::disallowed_methods,
    reason = "编译只在本机进行，见 `compiles_here`"
)]
pub fn compile(
    shell: &Shell,
    job: &Job,
    cancel: &Cancel,
    progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Option<Compiled> {
    let started = std::time::SystemTime::now();
    if let Some(out) = job.pdf.parent() {
        let _ = std::fs::create_dir_all(out);
    }
    let mut command = shell.command(&job.program);
    // `tectonic` 的进度和带文件名的错误在标准错误里。
    command
        .args(&job.args)
        .current_dir(&job.dir)
        .stderr(std::process::Stdio::piped());
    let reader = std::sync::Mutex::new(Reader::default());
    let heard = Arc::new(Mutex::new(std::time::Instant::now()));
    let last = Arc::clone(&heard);
    let watch: process::Watch = Arc::new(move |bytes: &[u8]| {
        if let Ok(mut at) = last.lock() {
            *at = std::time::Instant::now();
        }
        let found = (reader.lock().ok()).and_then(|mut r| r.feed(&String::from_utf8_lossy(bytes)));
        if let Some(found) = found {
            progress(found);
        }
    });
    // 没有输出太久时只停下程序，仍然报告结果。
    let (inner, stalled) = (Cancel::default(), Arc::new(AtomicBool::new(false)));
    // 程序结束时关掉通道，看守线程随即退出。
    let (finished, wake) = std::sync::mpsc::channel::<()>();
    let watchdog = {
        let (outer, inner, stalled) = (cancel.clone(), inner.clone(), Arc::clone(&stalled));
        std::thread::spawn(move || {
            let step = std::time::Duration::from_millis(200);
            while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = wake.recv_timeout(step) {
                let quiet = heard.lock().is_ok_and(|at| at.elapsed() > STALL);
                if quiet {
                    stalled.store(true, Ordering::Relaxed);
                }
                if quiet || outer.is_cancelled() {
                    inner.cancel();
                    return;
                }
            }
        })
    };
    let run = process::run_watched(&mut command, Vec::new(), COMPILE_LIMIT, &inner, Some(watch));
    drop(finished);
    let _ = watchdog.join();
    let output = match run {
        Ok(output) => output,
        Err(RunError::Cancelled) if !stalled.load(Ordering::Relaxed) => return None,
        Err(RunError::Cancelled) => {
            let minutes = STALL.as_secs() / 60;
            return Some(Compiled {
                pdf: false,
                problems: Vec::new(),
                failure: Some(winc_i18n::t!("editor-latex-stalled", minutes = minutes)),
            });
        }
        Err(e) => {
            return Some(Compiled {
                pdf: false,
                problems: Vec::new(),
                failure: Some(e.to_string()),
            });
        }
    };
    let log = std::fs::read(&job.log).unwrap_or_default();
    let mut problems = problems(&String::from_utf8_lossy(&log), &job.dir);
    // 日志没写文件时取标准错误里的 `error: 文件:行: 信息`。
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut named: Vec<Problem> = Vec::new();
    for found in (stderr.lines())
        .filter_map(|l| l.strip_prefix("error: "))
        .filter_map(|l| file_line_error(l, &job.dir))
    {
        if !named.contains(&found) {
            named.push(found);
        }
    }
    if !named.is_empty() && problems.iter().all(|p| p.file.is_none()) {
        problems = named;
    }
    // 还不知道文件的，是主文件的错误。
    for problem in &mut problems {
        problem.file.get_or_insert_with(|| job.root.clone());
    }
    let fresh = std::fs::metadata(&job.pdf)
        .and_then(|m| m.modified())
        .is_ok_and(|at| at >= started);
    // `latexmk` 没有改写 PDF 时，成功即可显示。
    let pdf = job.pdf.exists() && (output.status.success() || fresh);
    let failure = (!output.status.success() && problems.is_empty()).then(|| {
        let text = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last = text
            .lines()
            .chain(stdout.lines())
            .rev()
            .find(|l| !l.trim().is_empty());
        last.unwrap_or_default().trim().to_owned()
    });
    Some(Compiled {
        pdf,
        problems,
        failure,
    })
}

/// 读出编译日志 `log` 中的错误，文件按工作目录 `dir` 解析。
pub fn problems(log: &str, dir: &Path) -> Vec<Problem> {
    const WRAP: usize = 79;
    let lines: Vec<&str> = log.lines().collect();
    let joined = |i: usize| {
        let mut text = lines[i].to_owned();
        let mut k = i;
        while lines[k].chars().count() == WRAP && k + 1 < lines.len() {
            k += 1;
            text.push_str(lines[k]);
        }
        text
    };
    let mut out: Vec<Problem> = Vec::new();
    for i in 0..lines.len() {
        let line = joined(i);
        let found = if let Some(rest) = line.strip_prefix("! ") {
            let number = lines[i + 1..].iter().take(12).find_map(|l| {
                l.strip_prefix("l.")?
                    .split(|c: char| !c.is_ascii_digit())
                    .next()?
                    .parse()
                    .ok()
            });
            Some(Problem {
                file: None,
                line: number,
                message: rest.trim().to_owned(),
            })
        } else {
            file_line_error(&line, dir)
        };
        if let Some(problem) = found
            && !out.contains(&problem)
        {
            out.push(problem);
            if out.len() == MAX_PROBLEMS {
                break;
            }
        }
    }
    out
}

/// 读出 `文件:行: 信息` 这样的一行。
fn file_line_error(line: &str, dir: &Path) -> Option<Problem> {
    let (file, rest) = [
        ".tex:", ".sty:", ".cls:", ".ltx:", ".dtx:", ".bib:", ".bbl:",
    ]
    .iter()
    .find_map(|ext| {
        line.find(ext)
            .map(|i| (&line[..i + ext.len() - 1], &line[i + ext.len()..]))
    })?;
    let (number, message) = rest.split_once(": ")?;
    let number: usize = number.parse().ok()?;
    let file = file.trim();
    // `(./main.tex` 是日志里打开文件的记录，不是错误。
    if file.is_empty() || file.starts_with('(') {
        return None;
    }
    let path = Path::new(file.strip_prefix("./").unwrap_or(file));
    Some(Problem {
        file: Some(normalize(&dir.join(path))),
        line: Some(number),
        message: message.trim().to_owned(),
    })
}

#[cfg(test)]
mod tests;
