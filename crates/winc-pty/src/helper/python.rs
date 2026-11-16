//! Windows 原生交互式 shell 的 Python 环境：启动前将项目解释器目录加入环境。
//!
//! 仅探测启动目录中的 `.venv`、`venv`；不读取激活脚本，不影响 WSL、SSH 或运行配置。

use std::path::Path;

/// 环境变量表，每项是 UTF-8 编码的（键，值）。
type Env = Vec<(Vec<u8>, Vec<u8>)>;

/// 为没有参数的 Windows 原生 shell 加载启动目录中的 Python 环境。
///
/// 应在终端辅助进程中调用：最多检查两个环境的配置文件与解释器。已有活动环境时保留原值。
pub(super) fn activate(cwd: &[u8], program: &[u8], args: &[Vec<u8>], env: &mut Env) {
    let name = program
        .rsplit(|&b| matches!(b, b'/' | b'\\'))
        .next()
        .unwrap_or(program);
    let shell = [b"cmd.exe".as_slice(), b"powershell.exe", b"pwsh.exe"];
    if !args.is_empty() || !shell.iter().any(|shell| name.eq_ignore_ascii_case(shell)) {
        return;
    }
    // `WINC_PYTHON=0` 表示用户没有允许为这个目录加载环境；原生 shell 没有集成脚本删除它。
    let allowed = !env
        .iter()
        .any(|(key, value)| key == b"WINC_PYTHON" && value == b"0");
    env.retain(|(key, _)| key != b"WINC_PYTHON");
    if !allowed {
        return;
    }
    if [b"VIRTUAL_ENV".as_slice(), b"CONDA_PREFIX"]
        .iter()
        .any(|key| {
            env.iter()
                .any(|(name, value)| name.eq_ignore_ascii_case(key) && !value.is_empty())
        })
    {
        return;
    }
    let Ok(cwd) = std::str::from_utf8(cwd) else {
        return;
    };
    let root = Path::new(cwd);
    if !root.is_absolute() {
        return;
    }
    for name in [".venv", "venv"] {
        let venv = root.join(name);
        let bin = venv.join("Scripts");
        if !venv.join("pyvenv.cfg").is_file() || !bin.join("python.exe").is_file() {
            continue;
        }
        let Some(path) = bin.to_str() else { continue };
        let Some(venv) = venv.to_str() else { continue };
        let old = env
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(b"PATH"));
        let mut path = path.as_bytes().to_vec();
        if let Some((_, old)) = old.filter(|(_, old)| !old.is_empty()) {
            path.push(b';');
            path.extend_from_slice(old);
        }
        env.retain(|(key, _)| {
            ![
                b"PATH".as_slice(),
                b"PYTHONHOME",
                b"VIRTUAL_ENV",
                b"VIRTUAL_ENV_PROMPT",
            ]
            .iter()
            .any(|name| key.eq_ignore_ascii_case(name))
        });
        env.extend([
            (b"PATH".to_vec(), path),
            (b"VIRTUAL_ENV".to_vec(), venv.as_bytes().to_vec()),
            (
                b"VIRTUAL_ENV_PROMPT".to_vec(),
                format!("({name}) ").into_bytes(),
            ),
        ]);
        break;
    }
}

#[cfg(test)]
mod tests;
