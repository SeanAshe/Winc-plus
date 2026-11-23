//! shell 集成脚本的内容，以及在远端主机上使用它们的约定。
//!
//! 本机的 shell 由辅助进程在启动前写好脚本并注入。远端主机上的 shell 经 `ssh` 启动：脚本由
//! 本机的 winc 经已有的连接写到那台主机上的 [`REMOTE_DIR`]，终端里的 `ssh` 运行
//! [`remote_command`] 给出的命令，由其中的启动脚本按用户的登录 shell 注入。

use std::borrow::Cow;

/// zsh 的启动脚本：`.zshenv` 把 `ZDOTDIR` 恢复成用户原来的值，`winc.zsh` 安装钩子。
pub const ZSH: &[(&str, &str)] = &[
    ("zsh/.zshenv", include_str!("../shell/zshenv.zsh")),
    ("zsh/winc.zsh", include_str!("../shell/winc.zsh")),
];
/// bash 的启动脚本，由 `--rcfile` 读入。
pub const BASH: &[(&str, &str)] = &[("bash/winc.bash", include_str!("../shell/winc.bash"))];
/// fish 的启动脚本，放在 `vendor_conf.d` 里由 fish 自动读入。
pub const FISH: &[(&str, &str)] = &[(
    "fish/fish/vendor_conf.d/winc.fish",
    include_str!("../shell/winc.fish"),
)];
/// Python 环境的加载脚本：在用户配置读完后调整环境，并提供 `deactivate`。
pub const PYTHON: &[(&str, &str)] = &[
    ("python.sh", include_str!("../shell/python.sh")),
    ("python.fish", include_str!("../shell/python.fish")),
];
/// 按用户的登录 shell 设置环境变量和参数、启动它并注入上面各脚本的脚本；第一个参数是脚本所在
/// 的目录。用于 WSL 发行版和远端主机。
pub const LAUNCH: &[(&str, &str)] = &[("wsl.sh", include_str!("../shell/wsl.sh"))];

/// 远端主机上放集成脚本的目录，相对于用户的主目录。
pub const REMOTE_DIR: &str = ".winc-server/shell";

/// [`remote_command`] 中代表随机数的记号：辅助进程启动 `ssh` 之前把它换成为这个终端生成的
/// 随机数，脚本报告命令行时附带它。
pub const REMOTE_NONCE: &str = "@WINC_NONCE@";

/// 在远端启动带集成的登录 shell 的脚本，由 `sh -c` 运行：第一个参数是随机数，第二个是是否
/// 换上 winc 的提示符，第三个是是否加载 Python 环境（`0` 为否，与本机一样只在这时设置
/// `WINC_PROMPT`、`WINC_PYTHON`），第四个是要启动的 shell（空的是用户的登录 shell）。脚本不在
/// （尚未写好、主目录不可写）时启动不带集成的 shell。
/// 不含单引号，可以整个包在单引号里交给各种登录 shell。
const REMOTE_START: &str = concat!(
    r#"[ -n "$4" ] && export SHELL=$4; "#,
    r#"d=$HOME/.winc-server/shell; [ -r "$d/wsl.sh" ] && "#,
    r#"{ [ "$2" = 0 ] && export WINC_PROMPT=0; [ "$3" = 0 ] && export WINC_PYTHON=0; "#,
    r#"WINC_NONCE=$1 exec sh "$d/wsl.sh" "$d"; }; "#,
    r#"exec "${SHELL:-/bin/sh}" -l"#,
);

/// 返回脚本 `content` 写出去的内容：换行符统一为 LF，已是 LF 时不复制。
///
/// `include_str!` 内嵌的是检出时的字节，Windows 上 `core.autocrlf=true` 时检出为 CRLF，而读
/// 这些脚本的 shell（WSL、远端主机上的 `sh`、dash）把回车当作词的一部分，`case … in` 会解析
/// 失败。仓库的 `.gitattributes` 已让这些脚本总以 LF 检出，这里兜住规则之前的检出。
pub fn unix_lines(content: &str) -> Cow<'_, str> {
    match content.contains('\r') {
        true => Cow::Owned(content.replace("\r\n", "\n")),
        false => Cow::Borrowed(content),
    }
}

/// 返回远端主机上各个集成脚本的相对路径和内容（换行符为 LF，见 [`unix_lines`]），写到那台
/// 主机的 [`REMOTE_DIR`] 之下。
pub fn remote() -> impl Iterator<Item = (&'static str, Cow<'static, str>)> {
    [ZSH, BASH, FISH, PYTHON, LAUNCH]
        .into_iter()
        .flatten()
        .map(|&(name, content)| (name, unix_lines(content)))
}

/// 返回在远端启动带集成的登录 shell 的命令，接在 `cd '目录' &&` 之后交给 `ssh`；`prompt` 为
/// 假时不换提示符，`python` 为假时不加载 Python 环境，`shell` 是要启动的 shell 在那台主机上
/// 的路径（`None` 是用户的登录 shell）。其中的 [`REMOTE_NONCE`] 由辅助进程替换。
pub fn remote_command(prompt: bool, python: bool, shell: Option<&str>) -> String {
    let (prompt, python) = (u8::from(prompt), u8::from(python));
    let shell = single_quoted(shell.unwrap_or_default());
    format!("exec sh -c '{REMOTE_START}' winc {REMOTE_NONCE} {prompt} {python} {shell}")
}

/// 把 `text` 包在单引号里，其中的单引号写成 `'\''`：各种登录 shell 都按字面处理。
pub fn single_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_written_with_unix_endings() {
        // 以 CRLF 检出时内嵌的脚本：写出去的是 LF；已是 LF 的不复制。
        let crlf = "case ${shell##*/} in\r\nzsh)\r\n";
        assert_eq!(unix_lines(crlf), "case ${shell##*/} in\nzsh)\n");
        assert!(matches!(unix_lines("a\nb\n"), Cow::Borrowed(_)));
        for (_, content) in remote() {
            assert!(!content.contains('\r'));
        }
    }

    #[test]
    fn the_remote_command_survives_any_login_shell() {
        // 整个脚本包在单引号里：其中不能有单引号；`!` 在 tcsh 的单引号里仍然引出历史。
        assert!(!REMOTE_START.contains(['\'', '!', '\n']));
        let command = remote_command(false, true, None);
        assert!(command.starts_with("exec sh -c '[ -n \"$4\" ] && export SHELL=$4; d=$HOME/"));
        assert!(command.ends_with("' winc @WINC_NONCE@ 0 1 ''"));
        assert!(remote_command(true, false, None).ends_with("' winc @WINC_NONCE@ 1 0 ''"));
        // 选定的 shell 作为第四个参数，路径中的单引号不结束引号。
        let chosen = remote_command(true, true, Some("/opt/it's/fish"));
        assert!(
            chosen.ends_with(r"' winc @WINC_NONCE@ 1 1 '/opt/it'\''s/fish'"),
            "{chosen}"
        );
        assert!(REMOTE_START.contains(REMOTE_DIR));
        let names: Vec<_> = remote().map(|(name, _)| name).collect();
        assert!(names.contains(&"wsl.sh") && names.contains(&"zsh/.zshenv"));
        assert_eq!(names.len(), 7);
    }
}
