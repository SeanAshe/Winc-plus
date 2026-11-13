//! 运行在远端主机上的服务进程。
//!
//! 由本机的 winc 经 `ssh` 启动，从标准输入读取请求，把应答写到标准输出（见
//! `winc_backend::remote::serve`）。不监听端口，不写任何文件；标准输入结束（连接断开）时
//! 退出。目前只支持 Unix 主机。
//!
//! 以 `watch <目录>` 为参数时改为监视该目录，把变化写到标准输出（见 `winc_watch`）：由服务
//! 进程在需要监视时再次运行自身，连接断开或不再需要时结束它。
//!
//! 环境中有 `WINC_ASKPASS_SOCKET` 时是被 `git` 或 `ssh` 当作 askpass 程序运行：把参数中的
//! 提示交给启动那条命令的服务进程，由它转给本机的 winc 询问用户，再把答复写到标准输出。

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(unix)]
    if std::env::var_os(winc_backend::remote::ASKPASS_SOCKET).is_some() {
        return answer();
    }
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|arg| arg == "watch") {
        let Some(root) = args.next().map(std::path::PathBuf::from) else {
            return ExitCode::FAILURE;
        };
        return match winc_watch::run(&root, false) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("winc-server: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(unix)]
    {
        // 输出由多个线程共用，不能持有标准输出的锁；每一帧写完都会刷新。
        let input = std::io::BufReader::new(std::io::stdin().lock());
        let output = std::io::BufWriter::new(std::io::stdout());
        match winc_backend::remote::serve(input, output) {
            Ok(()) => ExitCode::SUCCESS,
            // 不合协议的请求或输入输出错误：没有可以应答的对象，直接结束。
            Err(_) => ExitCode::FAILURE,
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("winc-server: this platform is not supported");
        ExitCode::FAILURE
    }
}

/// 作为 askpass 程序：提示是第一个参数，答复写到标准输出并以换行结束；用户没有答复、或无法
/// 联系服务进程时以失败状态退出，`git` 和 `ssh` 按用户取消处理。
#[cfg(unix)]
fn answer() -> ExitCode {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;

    let prompt = std::env::args_os().nth(1).unwrap_or_default();
    let Ok(Some(mut reply)) = winc_backend::remote::answer_from_server(prompt.as_bytes()) else {
        return ExitCode::FAILURE;
    };
    reply.push(b'\n');
    match std::io::stdout().write_all(&reply) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
