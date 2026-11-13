//! shell 集成脚本的内容，以及在远端主机上使用它们的约定。
//!
//! 本机的 shell 由辅助进程在启动前写好脚本并注入。远端主机上的 shell 经 `ssh` 启动：脚本由
//! 本机的 winc 经已有的连接写到那台主机上的 [`REMOTE_DIR`]，终端里的 `ssh` 运行
//! [`remote_command`] 给出的命令，由其中的启动脚本按用户的登录 shell 注入。

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
/// 按用户的登录 shell 设置环境变量和参数、启动它并注入上面各脚本的脚本；第一个参数是脚本所在
/// 的目录。用于 WSL 发行版和远端主机。
pub const LAUNCH: &[(&str, &str)] = &[("wsl.sh", include_str!("../shell/wsl.sh"))];

/// 远端主机上放集成脚本的目录，相对于用户的主目录。
pub const REMOTE_DIR: &str = ".winc-server/shell";

/// [`remote_command`] 中代表随机数的记号：辅助进程启动 `ssh` 之前把它换成为这个终端生成的
/// 随机数，脚本报告命令行时附带它。
pub const REMOTE_NONCE: &str = "@WINC_NONCE@";

/// 在远端启动带集成的登录 shell 的脚本，由 `sh -c` 运行：第一个参数是随机数，第二个是是否
/// 换上 winc 的提示符（`0` 为不换，与本机一样只在这时设置 `WINC_PROMPT`）。脚本不在（尚未写好、主目录不可写）时启动不带集成的 shell。
/// 不含单引号，可以整个包在单引号里交给各种登录 shell。
const REMOTE_START: &str = concat!(
    r#"d=$HOME/.winc-server/shell; [ -r "$d/wsl.sh" ] && "#,
    r#"{ [ "$2" = 0 ] && export WINC_PROMPT=0; WINC_NONCE=$1 exec sh "$d/wsl.sh" "$d"; }; "#,
    r#"exec "${SHELL:-/bin/sh}" -l"#,
);

/// 返回远端主机上各个集成脚本的相对路径和内容，写到那台主机的 [`REMOTE_DIR`] 之下。
pub fn remote() -> impl Iterator<Item = (&'static str, &'static str)> {
    [ZSH, BASH, FISH, LAUNCH].into_iter().flatten().copied()
}

/// 返回在远端启动带集成的登录 shell 的命令，接在 `cd '目录' &&` 之后交给 `ssh`；`prompt` 为
/// 假时不换提示符。其中的 [`REMOTE_NONCE`] 由辅助进程替换。
pub fn remote_command(prompt: bool) -> String {
    let prompt = u8::from(prompt);
    format!("exec sh -c '{REMOTE_START}' winc {REMOTE_NONCE} {prompt}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remote_command_survives_any_login_shell() {
        // 整个脚本包在单引号里：其中不能有单引号；`!` 在 tcsh 的单引号里仍然引出历史。
        assert!(!REMOTE_START.contains(['\'', '!', '\n']));
        let command = remote_command(false);
        assert!(command.starts_with("exec sh -c 'd=$HOME/.winc-server/shell;"));
        assert!(command.ends_with("' winc @WINC_NONCE@ 0"));
        assert!(REMOTE_START.contains(REMOTE_DIR));
        let names: Vec<_> = remote().map(|(name, _)| name).collect();
        assert!(names.contains(&"wsl.sh") && names.contains(&"zsh/.zshenv"));
        assert_eq!(names.len(), 5);
    }
}
