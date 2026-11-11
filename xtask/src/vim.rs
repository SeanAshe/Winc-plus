//! `cargo xtask vim-fixtures`：用本机的 `vim` 执行对照测试的用例，录制执行结果。
//!
//! 用例写在 `crates/winc-vim/fixtures/*.cases` 中。每个用例启动一个不加载任何配置的 `vim` 进程，
//! 以 `feedkeys(…, "tx")` 送入按键，结果文本、光标与寄存器写入同名的 `.rec` 文件；测试只读取
//! `.rec` 文件。无界面执行结束时 `vim` 自动离开插入模式，录制结果不含最终的模式；按键出错后
//! 余下的按键仍然执行。

mod cases;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use winc_vim::fixture::{self, Case, Position, Register, RegisterKind, State};
use winc_vim::{Code, Key, notation};

use crate::root;

/// 默认使用的 `vim`。
const DEFAULT_VIM: &str = "/usr/bin/vim";

/// 录制基准的版本号前缀：只接受 9.1 的各个补丁版本。
const BASELINE: &str = "9.1.";

/// 单个 `vim` 进程的最长运行时间。
///
/// 在 Apple M4 Pro 上实测，一个用例约 13 ms；超过 10 秒说明按键使 `vim` 停在等待输入的状态。
const TIMEOUT: Duration = Duration::from_secs(10);

/// 录制的寄存器，按写入文件的顺序排列：无名、编号、小删除、具名、最近插入、最近命令行、最近查找。
const REGISTERS: &str = "\"0123456789-abcdefghijklmnopqrstuvwxyz.:/";

/// 录制文件的第一行。
const HEADER: &str =
    "# 由 `cargo xtask vim-fixtures` 录制，不手工修改；用例写在同名的 .cases 文件中。\n";

/// `cargo xtask vim-fixtures [--check] [--full] [--vim 路径]`：生成并录制全部用例文件。
///
/// 带 `--check` 时不写文件，只比较录制结果与现有的 `.rec` 文件，不一致时返回 `Ok(false)`。
/// 带 `--full` 时生成全部组合并录制到 `target/vim-fixtures-full`，用于开发时的完整核对：
/// `WINC_VIM_FIXTURES=target/vim-fixtures-full cargo test -p winc-vim --test fixtures`。
///
/// # Errors
///
/// 参数无法识别、没有用例文件、`vim` 无法运行或版本不是 9.1、用例有误、文件无法读写时返回原因。
pub fn fixtures(args: &[String]) -> Result<bool, String> {
    let mut check = false;
    let mut full = false;
    let mut program = DEFAULT_VIM;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--full" => full = true,
            "--vim" => program = args.next().ok_or("--vim 后面要跟路径")?,
            other => return Err(format!("vim-fixtures 不认识参数 {other}")),
        }
    }

    // 全部组合不放进仓库：写到 `target` 下，核对时以环境变量 `WINC_VIM_FIXTURES` 指向该目录。
    let dir = if full {
        root().join("target/vim-fixtures-full")
    } else {
        root().join("crates/winc-vim/fixtures")
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;
    // 生成的用例先写成用例文件，再与手写的用例文件一并录制。
    for group in cases::generated(full) {
        let path = dir.join(format!("{}.cases", group.name));
        let text = format!(
            "# 由 `cargo xtask vim-fixtures` 生成，不手工修改。{}\n{}",
            group.about,
            fixture::write(&group.file)
        );
        if !check && std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            continue;
        }
        if check {
            if !std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
                println!("{}.cases 与生成的用例不一致", group.name);
                return Ok(false);
            }
        } else {
            std::fs::write(&path, text).map_err(|e| format!("无法写入 {}：{e}", path.display()))?;
        }
    }
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("无法读取 {}：{e}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "cases"))
        .collect();
    sources.sort();
    if sources.is_empty() {
        return Err(format!("{} 中没有 .cases 文件", dir.display()));
    }

    let version = Vim::new(program, 0)?.version()?;
    if !version.starts_with(BASELINE) {
        return Err(format!("录制基准是 vim 9.1，{program} 的版本是 {version}"));
    }

    let mut current = true;
    for source in &sources {
        let name = source.file_name().unwrap_or_default().to_string_lossy();
        let text = std::fs::read_to_string(source).map_err(|e| format!("无法读取 {name}：{e}"))?;
        let mut file = fixture::parse(&text).map_err(|e| format!("{name}：{e}"))?;
        let states = record_all(program, &file.cases).map_err(|e| format!("{name} 的{e}"))?;
        for (case, state) in file.cases.iter_mut().zip(states) {
            case.result = Some(state);
        }
        file.vim = Some(version.clone());
        let recorded = format!("{HEADER}{}", fixture::write(&file));

        let target = source.with_extension("rec");
        let count = file.cases.len();
        if std::fs::read_to_string(&target).is_ok_and(|existing| existing == recorded) {
            println!("{name}：{count} 个用例，录制结果无变化");
        } else if check {
            current = false;
            println!("{name}：{count} 个用例，录制结果与 .rec 文件不一致");
        } else {
            std::fs::write(&target, recorded)
                .map_err(|e| format!("无法写入 {}：{e}", target.display()))?;
            println!("{name}：{count} 个用例，已写入 .rec 文件（vim {version}）");
        }
    }
    Ok(current)
}

/// 录制 `cases` 中的每个用例，返回各用例的结果；用例分给若干线程，各自启动 `vim` 进程。
///
/// # Errors
///
/// 任何一个用例录制失败时返回「用例 名字：原因」。
fn record_all(program: &str, cases: &[Case]) -> Result<Vec<State>, String> {
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk = cases.len().div_ceil(threads).max(1);
    let results: Vec<Result<Vec<State>, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (cases.chunks(chunk).enumerate())
            .map(|(index, chunk)| {
                scope.spawn(move || {
                    let vim = Vim::new(program, index + 1)?;
                    (chunk.iter())
                        .map(|case| {
                            (vim.record(case)).map_err(|e| format!("用例 {}：{e}", case.name))
                        })
                        .collect()
                })
            })
            .collect();
        (handles.into_iter())
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("录制线程异常结束".into()))
            })
            .collect()
    });
    let mut states = Vec::with_capacity(cases.len());
    for result in results {
        states.extend(result?);
    }
    Ok(states)
}

/// 一个可以执行脚本的 `vim`，以及存放脚本与结果的临时目录；销毁时删除该目录。
struct Vim {
    /// `vim` 的路径或命令名。
    program: String,
    /// 临时目录，每个录制线程一个。
    dir: PathBuf,
}

impl Drop for Vim {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Vim {
    /// 创建临时目录；`index` 区分同一进程中的各个录制线程。
    fn new(program: &str, index: usize) -> Result<Self, String> {
        let dir =
            std::env::temp_dir().join(format!("winc-vim-fixtures-{}-{index}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;
        Ok(Self {
            program: program.to_owned(),
            dir,
        })
    }

    /// 返回 `vim` 的版本号，例如 `9.1.1752`。
    fn version(&self) -> Result<String, String> {
        let output = self.dir.join("version.txt");
        let script = format!(
            "call writefile([string(v:versionlong)], {})\nqa!\n",
            quoted_path(&output)
        );
        let raw = self.execute(&script, &output)?;
        let text = String::from_utf8_lossy(&raw);
        version_string(text.trim()).ok_or_else(|| format!("无法识别 vim 的版本：{text}"))
    }

    /// 执行用例 `case`，返回 `vim` 中的结果。
    fn record(&self, case: &Case) -> Result<State, String> {
        let input = self.dir.join("input.txt");
        let output = self.dir.join("output.txt");
        let cursor = byte_cursor(&case.text, case.cursor)?;
        let keys = vim_keys(&notation::parse(&case.keys))?;
        std::fs::write(&input, case.text.join("\n"))
            .map_err(|e| format!("无法写入 {}：{e}", input.display()))?;
        let raw = self.execute(&script(&input, &output, cursor, &keys), &output)?;
        parse_output(&raw)
    }

    /// 以不加载任何配置的无界面方式执行脚本 `script`，返回脚本写出的文件 `output` 的内容。
    ///
    /// 进程的退出状态不作判断：按键中的命令报错（例如查找不到模式）时退出状态为 1，结果仍然有效。
    fn execute(&self, script: &str, output: &Path) -> Result<Vec<u8>, String> {
        let path = self.dir.join("script.vim");
        std::fs::write(&path, script).map_err(|e| format!("无法写入 {}：{e}", path.display()))?;
        let _ = std::fs::remove_file(output);

        let mut child = Command::new(&self.program)
            .args(["-u", "NONE", "-i", "NONE", "-N", "-n", "-es", "-S"])
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("无法运行 {}：{e}", self.program))?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() > TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("vim 运行超过 {} 秒，已终止", TIMEOUT.as_secs()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(1)),
                Err(e) => return Err(format!("无法等待 vim 结束：{e}")),
            }
        }
        std::fs::read(output).map_err(|e| format!("vim 没有写出结果：{e}"))
    }
}

/// 把 `v:versionlong` 的值（主版本、两位次版本、四位补丁号相连，如 `9011752`）写成 `9.1.1752`。
fn version_string(long: &str) -> Option<String> {
    let long: u64 = long.parse().ok()?;
    Some(format!(
        "{}.{}.{}",
        long / 1_000_000,
        long / 10_000 % 100,
        long % 10_000
    ))
}

/// 把用例的光标位置换算成 `cursor()` 的参数：从 1 起的行号与行内的字节列号。
///
/// # Errors
///
/// 文本的行内含换行符，或光标不在某个字符上（空行只能在第 0 列）时返回原因。
fn byte_cursor(text: &[String], cursor: Position) -> Result<(usize, usize), String> {
    if text.iter().any(|line| line.contains('\n')) {
        return Err("text 的一行之内不能有换行符".into());
    }
    let line = text.get(cursor.line).ok_or("光标的行号超出文本")?;
    let col = match line.char_indices().nth(cursor.col) {
        Some((byte, _)) => byte,
        None if cursor.col == 0 => 0,
        None => return Err("光标的列号超出该行".into()),
    };
    Ok((cursor.line + 1, col + 1))
}

/// 把按键序列写成 Vim 脚本双引号字符串的内容：功能键与带修饰键的按键写作 `\<名字>`。
///
/// # Errors
///
/// 带修饰键的 `"` 与 `\` 无法写入双引号字符串，返回原因。
fn vim_keys(keys: &[Key]) -> Result<String, String> {
    let mut out = String::new();
    for key in keys {
        match key.code() {
            Code::Char(c @ ('"' | '\\')) if key.mods().is_empty() => {
                out.push('\\');
                out.push(c);
            }
            Code::Char('"' | '\\') => return Err(format!("无法录制按键 {key}")),
            Code::Char(c) if key.mods().is_empty() && c != '<' && c != ' ' => out.push(c),
            // 其余按键的记法都以 `<` 开头，与脚本中的写法相同。
            _ => {
                let _ = write!(out, "\\{key}");
            }
        }
    }
    Ok(out)
}

/// 返回 Vim 脚本中表示 `path` 的单引号字符串。
fn quoted_path(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

/// 返回执行一个用例的脚本：从 `input` 读入文本，把光标放到 `cursor`（行号与字节列号，都从 1 起），
/// 送入 `keys`（[`vim_keys`] 的结果），把结果写入 `output`。
///
/// 结果每行以一个字母开头：`C 行号 字节列号`、`T 文本行`、`R 寄存器名 类型 行数` 及随后的
/// `L 内容行`，最后是 `E`。
fn script(input: &Path, output: &Path, cursor: (usize, usize), keys: &str) -> String {
    format!(
        r#"set encoding=utf-8
scriptencoding utf-8
set nomore
set undolevels=-1
call setline(1, readfile({input}, 'b'))
set undolevels=1000
call cursor({line}, {col})
call feedkeys("{keys}", 'tx')
let s:out = ['C ' . line('.') . ' ' . col('.')]
for s:line in getline(1, '$')
  call add(s:out, 'T ' . s:line)
endfor
for s:name in split('{REGISTERS}', '\zs')
  let s:info = getreginfo(s:name)
  if !empty(s:info) && !empty(s:info.regcontents)
    let s:type = s:info.regtype
    if s:type[0] ==# "\<C-v>"
      let s:type = 'b' . s:type[1:]
    endif
    call add(s:out, 'R ' . s:name . ' ' . s:type . ' ' . len(s:info.regcontents))
    for s:line in s:info.regcontents
      call add(s:out, 'L ' . s:line)
    endfor
  endif
endfor
call add(s:out, 'E')
call writefile(s:out, {output})
qa!
"#,
        input = quoted_path(input),
        output = quoted_path(output),
        line = cursor.0,
        col = cursor.1,
    )
}

/// 解析 [`script`] 写出的结果。
///
/// # Errors
///
/// 结果不是 UTF-8、不完整或格式不符时返回原因。
fn parse_output(raw: &[u8]) -> Result<State, String> {
    // 最近插入的文本中的功能键是 `vim` 内部的字节，不是 UTF-8：该行记作 [`fixture::OPAQUE`]。
    let mut text = String::with_capacity(raw.len());
    for (index, line) in raw.split(|byte| *byte == b'\n').enumerate() {
        if index > 0 {
            text.push('\n');
        }
        match std::str::from_utf8(line) {
            Ok(line) => text.push_str(line),
            Err(_) if line.starts_with(b"L ") => {
                text.push_str("L ");
                text.push_str(fixture::OPAQUE);
            }
            Err(e) => return Err(format!("结果不是 UTF-8：{e}")),
        }
    }
    let text = text.as_str();
    // 文本行内可以有 CR，只按 LF 分行。
    let mut lines = text.split('\n');
    let malformed = |line: &str| format!("结果的格式不符：{line}");

    let first = lines.next().unwrap_or_default();
    let mut numbers =
        (first.strip_prefix("C ").unwrap_or_default().split(' ')).map(str::parse::<usize>);
    let (Some(Ok(line)), Some(Ok(col)), None) = (numbers.next(), numbers.next(), numbers.next())
    else {
        return Err(malformed(first));
    };

    let mut state = State::default();
    loop {
        let line = lines.next().ok_or("结果不完整")?;
        if let Some(text) = line.strip_prefix("T ") {
            state.text.push(text.to_owned());
        } else if let Some(register) = line.strip_prefix("R ") {
            let mut chars = register.chars();
            let (Some(name), Some(' ')) = (chars.next(), chars.next()) else {
                return Err(malformed(line));
            };
            let (kind, count) = chars
                .as_str()
                .split_once(' ')
                .ok_or_else(|| malformed(line))?;
            let kind = match (kind, kind.strip_prefix('b').map(str::parse::<usize>)) {
                ("v", _) => RegisterKind::Charwise,
                ("V", _) => RegisterKind::Linewise,
                (_, Some(Ok(width))) => RegisterKind::Blockwise(width),
                _ => return Err(malformed(line)),
            };
            let count: usize = count.parse().map_err(|_| malformed(line))?;
            let mut contents = Vec::new();
            for _ in 0..count {
                let line = lines.next().ok_or("结果不完整")?;
                contents.push(
                    line.strip_prefix("L ")
                        .ok_or_else(|| malformed(line))?
                        .to_owned(),
                );
            }
            state.registers.push(Register {
                name,
                kind,
                lines: contents,
            });
        } else if line == "E" {
            break;
        } else {
            return Err(malformed(line));
        }
    }

    // `vim` 报告从 1 起的行号与字节列号，换算成从 0 起的行号与字符列号。
    let current = (line.checked_sub(1))
        .and_then(|line| state.text.get(line))
        .ok_or("结果中光标的行号超出文本")?;
    let prefix = (col.checked_sub(1))
        .and_then(|col| current.get(..col))
        .ok_or("结果中光标的列号不在字符边界上")?;
    state.cursor = Position {
        line: line - 1,
        col: prefix.chars().count(),
    };
    Ok(state)
}

#[cfg(test)]
mod tests;
