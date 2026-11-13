//! shell 集成的端到端测试：启动真实的 zsh、bash、fish，检查注入之后用户自己的配置仍然生效、
//! 环境里不留 winc 的变量，以及命令块的边界、退出码、命令行、目录和时长。
//!
//! 机器上没有安装的 shell 跳过对应测试。

#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::Client;
use winc_pty::protocol::{Block, FromHelper, Marks, ToHelper};

/// 测试用的临时目录，删除实例时连同内容一起删掉。
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("winc-pty-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        // macOS 的临时目录位于 `/var`，该路径是指向 `/private/var` 的符号链接；shell 报告
        // 的目录是解析后的真实路径。
        #[allow(clippy::disallowed_methods)] // 只在 Unix 上运行，没有 Windows 的路径前缀
        Temp(dir.canonicalize().unwrap())
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn find(shell: &str) -> Option<String> {
    ["/bin", "/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"]
        .iter()
        .map(|dir| format!("{dir}/{shell}"))
        .find(|path| Path::new(path).exists())
}

fn env(home: &Path, extra: &[(&str, &str)]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut env = vec![
        (b"HOME".to_vec(), home.to_str().unwrap().as_bytes().to_vec()),
        (b"PATH".to_vec(), b"/usr/bin:/bin:/usr/sbin:/sbin".to_vec()),
        (b"LANG".to_vec(), b"en_US.UTF-8".to_vec()),
        (b"BASH_SILENCE_DEPRECATION_WARNING".to_vec(), b"1".to_vec()),
        // Debian 与 Ubuntu 的 `/etc/zsh/zshrc` 会运行 `compinit`；目录权限不符合它的要求时
        // 会停下来提问，测试等不到提示符。这一步与被测行为无关，跳过。
        (b"skip_global_compinit".to_vec(), b"1".to_vec()),
    ];
    for (k, v) in extra {
        env.push((k.as_bytes().to_vec(), v.as_bytes().to_vec()));
    }
    env
}

struct Shell {
    client: Client,
    blocks: Vec<Block>,
}

impl Shell {
    fn start(program: &str, login: bool, env: Vec<(Vec<u8>, Vec<u8>)>, dir: &Path) -> Shell {
        let mut client = Client::start();
        client.spawn_shell(1, program, login, env, dir);
        Shell {
            client,
            blocks: Vec::new(),
        }
    }

    fn input(&mut self, text: &str) {
        self.client.send(ToHelper::Input {
            tab: 1,
            bytes: text.as_bytes().to_vec(),
            rest: 0,
        });
    }

    /// 接收消息直到 `done`（参数为屏幕和已收到的块）返回 `true`，期间的块记录在 `blocks` 中。
    ///
    /// 调用时 `done` 已经返回 `true` 时不再等待。
    fn until(&mut self, mut done: impl FnMut(&Client, &[Block]) -> bool) {
        if done(&self.client, &self.blocks) {
            return;
        }
        let mut blocks = std::mem::take(&mut self.blocks);
        self.client.until(|c, m| {
            if let FromHelper::Blocks { blocks: b, .. } = m {
                blocks.clone_from(b);
            }
            done(c, &blocks)
        });
        self.blocks = blocks;
    }

    fn until_blocks(&mut self, mut done: impl FnMut(&[Block]) -> bool) {
        self.until(|_, b| done(b));
    }

    /// 运行一条命令，等到它结束、下一个提示符画出，返回该命令的块。
    fn run(&mut self, command: &str) -> Block {
        // 同一条命令可能已经运行过：只认新出现的块。新块在运行时已经带上提示符，`output` 仍为
        // `None`。
        let seen: Vec<u32> = self
            .blocks
            .iter()
            .filter(|b| b.output.is_some())
            .map(|b| b.id)
            .collect();
        let text = command.trim_start();
        let new = |b: &Block| b.text.trim_start() == text && !seen.contains(&b.id);
        self.input(&format!("{command}\r"));
        self.client.waiting = format!("命令「{command}」的块结束，结束行是提示符");
        // 块的消息不受额度限制，屏幕可能尚未更新：等到结束行在屏幕上是提示符。
        self.until(|c, blocks| {
            blocks
                .iter()
                .find(|b| new(b))
                .and_then(|b| b.end)
                .is_some_and(|end| {
                    c.screen
                        .get(end as usize)
                        .is_some_and(|l| l.marks.flags & Marks::PROMPT != 0)
                })
        });
        self.client.waiting.clear();
        self.blocks.iter().find(|b| new(b)).unwrap().clone()
    }

    /// 检查失败的命令之后直接回车只多出一个提示符，不多出一条命令。
    fn empty_enter_adds_no_command(&mut self) {
        self.run("false");
        let commands = |b: &[Block]| b.iter().filter(|b| b.output.is_some()).count();
        let before = commands(&self.blocks);
        let prompts = self.blocks.len();
        self.input("\r");
        self.until_blocks(|b| b.len() > prompts);
        std::thread::sleep(Duration::from_millis(200));
        self.run("echo after");
        assert_eq!(commands(&self.blocks), before + 1, "{:#?}", self.blocks);
    }

    /// 命令的输出文本（输出的第一行到结束行之前），每行去掉首尾空白后用换行符连接。
    fn output(&self, block: &Block) -> String {
        let (start, end) = (block.output.unwrap(), block.end.unwrap());
        (start..end)
            .map(|l| self.client.screen[l as usize].trimmed().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 检查退出码、命令行、目录、时长，以及环境中没有残留 winc 的变量。
    ///
    /// `expected_cwd` 是 shell 报告的当前目录；未注入时 shell 恢复的用户目录为 `none`。
    fn check(&mut self, expected_cwd: &str) {
        let ok = self.run("true");
        assert_eq!(ok.exit, Some(0), "{ok:?}");
        let failed = self.run("false");
        assert_eq!(failed.exit, Some(1), "{failed:?}");
        assert!(failed.prompt > ok.prompt);
        assert_eq!(ok.end, Some(failed.prompt), "块首尾相接");

        let slow = self.run("sleep 0.3");
        assert!(slow.duration.unwrap() >= 250, "{slow:?}");

        let echo = self.run("echo \"a;b  c\"");
        assert_eq!(echo.output.map(|o| o - echo.command), Some(1));
        assert_eq!(self.output(&echo), "a;b  c");

        // 命令输出的命令行没有 shell 的随机数，不替换 shell 报告的命令行。
        let forged = self.run("printf '\\033]133;C;cmdline=echo forged\\a'");
        assert_eq!(forged.exit, Some(0), "{forged:?}");

        // `printenv … || echo none` 在 bash、zsh、fish 里写法相同。
        let leak = self.run("env | grep -c WINC_ ; printenv ZDOTDIR || echo none");
        // 管道的成员退出得早时，bash 3.2 会报 `child setpgid` 失败。这条竞争的提示与被检查的环境
        // 无关，比较前去掉。
        let output = self.output(&leak);
        let output: Vec<&str> = output
            .lines()
            .filter(|line| !line.contains("child setpgid"))
            .collect();
        assert_eq!(output.join("\n"), format!("0\n{expected_cwd}"));

        let cd = self.run("cd /usr");
        assert_eq!(cd.cwd, b"/", "块记的是命令开始时的目录");
        let after = self.run("pwd");
        assert_eq!(after.cwd, b"/usr");
    }
}

#[test]
fn zsh_reads_the_users_zdotdir_and_reports_blocks() {
    let Some(zsh) = find("zsh") else { return };
    let t = Temp::new("zsh");
    let zdot = t.0.join("zdot");
    t.write("zdot/.zshenv", "export FROM_ZSHENV=1\n");
    t.write("zdot/.zshrc", "PS1='zz> '\nunsetopt prompt_sp\n");
    let mut sh = Shell::start(
        &zsh,
        true,
        env(&t.home(), &[("ZDOTDIR", zdot.to_str().unwrap())]),
        &t.0.join("shell"),
    );
    sh.client.until_text("zz>");
    let zshenv = sh.run("echo $FROM_ZSHENV");
    assert_eq!(sh.output(&zshenv), "1");
    // 用户的 ZDOTDIR 恢复了，子进程也看得到。
    sh.check(zdot.to_str().unwrap());
    // 命令行与提示符在同一行：标记 `B` 加在 `PS1` 末尾，命令行取输出开始的前一行。
    let last = sh.blocks.last().unwrap();
    assert_eq!(last.command, last.prompt);
}

#[test]
fn zsh_without_a_zdotdir_reads_home() {
    let Some(zsh) = find("zsh") else { return };
    let t = Temp::new("zsh-home");
    t.write("home/.zshrc", "PS1='hh> '\n");
    let mut sh = Shell::start(&zsh, false, env(&t.home(), &[]), &t.0.join("shell"));
    sh.client.until_text("hh>");
    sh.check("none");
    sh.empty_enter_adds_no_command();
}

#[test]
fn bash_as_a_login_shell_reads_the_profile() {
    let Some(bash) = find("bash") else { return };
    let t = Temp::new("bash");
    t.write(
        "home/.bash_profile",
        "PS1='[$?] bb$ '\nPROMPT_COMMAND='USER_PC=$?'\nHISTCONTROL=ignorespace\n",
    );
    let mut sh = Shell::start(&bash, true, env(&t.home(), &[]), &t.0.join("shell"));
    sh.client.until_text("bb$");
    sh.check("none");
    // 用户自己的 `PROMPT_COMMAND` 照常运行，看到的是命令的退出码；`PS1` 里的 `$?` 同样如此。
    // 屏幕上更早的行也有 `[1] bb$`（`check` 运行过失败的命令），只看这条命令之后的提示符。
    let failed = sh.run("false");
    let prompt = failed.end.unwrap() as usize;
    sh.client.waiting = format!("第 {prompt} 行的提示符以「[1] bb$」开头");
    sh.until(|c, _| c.screen[prompt].text.starts_with("[1] bb$"));
    sh.client.waiting.clear();
    let pc = sh.run("echo $USER_PC");
    assert_eq!(sh.output(&pc), "1");
    // 钩子不改变 `$_` 的值。
    let last = sh.run("true abc && echo $_");
    assert_eq!(sh.output(&last), "abc");
    // 未记入历史的命令（首字符为空格，`HISTCONTROL=ignorespace`）仍报告该命令自身的命令行。
    let hidden = sh.run(" echo hidden");
    assert_eq!(sh.output(&hidden), "hidden");
    sh.empty_enter_adds_no_command();
}

#[test]
fn bash_as_a_plain_shell_reads_bashrc() {
    let Some(bash) = find("bash") else { return };
    let t = Temp::new("bash-rc");
    t.write("home/.bashrc", "PS1='rc$ '\n");
    let mut sh = Shell::start(&bash, false, env(&t.home(), &[]), &t.0.join("shell"));
    sh.client.until_text("rc$");
    sh.check("none");
}

#[test]
fn fish_reports_blocks() {
    let Some(fish) = find("fish") else {
        eprintln!("没有 fish，跳过");
        return;
    };
    let t = Temp::new("fish");
    let mut sh = Shell::start(&fish, true, env(&t.home(), &[]), &t.0.join("shell"));
    sh.until_blocks(|b| !b.is_empty());
    std::thread::sleep(Duration::from_millis(200));
    sh.check("none");
}

/// 等到屏幕上最后一个 `❯` 所在的那一行以颜色 `color` 绘制。
fn wait_arrow(sh: &mut Shell, color: winc_pty::protocol::Color) {
    sh.until(|c, _| {
        let Some(line) = c.screen.iter().rev().find(|l| l.text.contains('❯')) else {
            return false;
        };
        let col: usize = line
            .text
            .chars()
            .take_while(|&ch| ch != '❯')
            .map(winc_pty::width::char_width)
            .sum();
        line.runs
            .iter()
            .rev()
            .find(|r| usize::from(r.start) <= col)
            .is_some_and(|r| r.style.fg == color)
    });
}

#[test]
fn zsh_without_a_prompt_of_its_own_gets_a_clear_one() {
    use winc_pty::protocol::Color;
    let Some(zsh) = find("zsh") else { return };
    let t = Temp::new("zsh-prompt");
    // 一个仓库：分支名从 `.git/HEAD` 读取。
    t.write("home/repo/.git/HEAD", "ref: refs/heads/feature-x\n");
    let mut sh = Shell::start(&zsh, true, env(&t.home(), &[]), &t.0.join("shell"));
    sh.client.until_text("❯");
    sh.check("none");
    let colors = sh.run("echo $CLICOLOR");
    assert_eq!(sh.output(&colors), "1", "ls 带颜色");
    sh.run("cd ~/repo");
    sh.until(|c, _| c.text().contains("feature-x"));
    wait_arrow(&mut sh, Color::Indexed(2));
    sh.run("false");
    // 命令失败后提示符变为红色（新提示符的绘制可能晚于块的消息）。
    wait_arrow(&mut sh, Color::Indexed(1));
}

#[test]
fn the_prompt_is_left_alone_when_turned_off() {
    let Some(zsh) = find("zsh") else { return };
    let t = Temp::new("zsh-plain");
    let mut sh = Shell::start(
        &zsh,
        true,
        env(&t.home(), &[("WINC_PROMPT", "0")]),
        &t.0.join("shell"),
    );
    sh.until_blocks(|b| !b.is_empty());
    let block = sh.run("echo $CLICOLOR-${WINC_PROMPT-unset}");
    assert_eq!(sh.output(&block), "-unset");
    assert!(!sh.client.text().contains('❯'));
}

#[test]
fn bash_without_a_prompt_of_its_own_gets_a_clear_one() {
    let Some(bash) = find("bash") else { return };
    let t = Temp::new("bash-prompt");
    t.write("home/repo/.git/HEAD", "0123456789abcdef\n");
    let mut sh = Shell::start(&bash, true, env(&t.home(), &[]), &t.0.join("shell"));
    sh.client.until_text("❯");
    sh.check("none");
    sh.run("cd ~/repo");
    sh.until(|c, _| c.text().contains("0123456"));
    sh.run("false");
    wait_arrow(&mut sh, winc_pty::protocol::Color::Indexed(1));
}

/// 两行的提示符（第一行用点填满整行宽）在改窄后由终端重排成两行，zsh 只上退一行重画。
///
/// 回归测试：终端重排与 zsh 重画都会留下提示符，只统计未重画的旧提示符时屏幕残留旧片段。
#[test]
fn zsh_redraws_a_full_width_prompt_once_after_a_resize() {
    use winc_pty::protocol::Size;
    let Some(zsh) = find("zsh") else { return };
    let t = Temp::new("zsh-resize");
    t.write(
        "home/.zshrc",
        "setopt prompt_subst\nunsetopt prompt_sp\n\
         _fill() { _line=TOP${(l:$((COLUMNS-6))::.:)}END }\n\
         precmd_functions+=(_fill)\n\
         TRAPWINCH() { _fill; zle && zle reset-prompt }\n\
         PS1='${_line}\npp> '\n",
    );
    let mut sh = Shell::start(&zsh, false, env(&t.home(), &[]), &t.0.join("shell"));
    sh.run("echo before");
    // `run` 在提示符开始绘制时即返回；先等整个提示符画完再改宽度，否则 zsh 仍在按旧宽度绘制。
    sh.until(|c, _| c.text().lines().rev().find(|l| !l.is_empty()) == Some("pp>"));
    let resize = |cols| ToHelper::Resize {
        tab: 1,
        size: Size {
            cols,
            rows: 100,
            width: cols * 8,
            height: 1600,
        },
    };
    // 宽度连续变化多次，每次等 zsh 按新宽度画完提示符后再变下一次：两次变化过于接近时，
    // zsh 重画与终端重排的先后顺序不确定，结果每次都不同，无法断言确定的行为。
    for cols in [70u16, 60, 75, 50] {
        sh.client.send(resize(cols));
        let line = format!("TOP{}END", ".".repeat(usize::from(cols) - 6));
        sh.until(|c, _| c.text().lines().any(|l| l == line));
    }
    sh.input("x");
    sh.until(|c, _| c.text().contains("pp> x"));
    std::thread::sleep(Duration::from_millis(200));
    sh.input("y");
    sh.until(|c, _| c.text().contains("pp> xy"));

    let text = sh.client.text();
    let tops = text.lines().filter(|l| l.contains("TOP")).count();
    // 一段是 `echo before` 之前的提示符，该命令已经执行完毕、不再重画；另一段是当前提示符。
    assert_eq!(tops, 2, "提示符多出来了：\n{text}");
    let last = text.lines().rev().find(|l| l.contains("TOP")).unwrap();
    assert_eq!(
        last,
        format!("TOP{}END", ".".repeat(44)),
        "按新宽度重画：\n{text}"
    );
    // 已执行命令之前的提示符属于普通历史，仍然随重排移动；当前提示符下方不留残留片段。
    let live: Vec<&str> = text.lines().skip_while(|l| *l != "before").collect();
    assert_eq!(live, ["before", last, "pp> xy"], "旧提示符的残段：\n{text}");
}
