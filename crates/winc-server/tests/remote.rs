//! 端到端：本机的文件操作经连接到达服务进程，结果与直接访问本机文件一致。
//!
//! 服务进程直接作为子进程启动，不经过 `ssh`；经 `ssh` 的测试需要一台可登录的主机，默认不运行
//! （见 [`over_ssh`]）。

#![cfg(unix)]
#![allow(
    clippy::disallowed_methods,
    reason = "测试只在 Unix 上运行，直接取临时目录的真实路径"
)]

use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use winc_backend::remote::{self, Connection};
use winc_backend::{Target, fs, virtual_path};

/// 测试中的 `ssh` 不向用户提问。
const BATCH: remote::Ssh = remote::Ssh {
    batch: true,
    keep_alive: false,
    connect_timeout: false,
    share: false,
};

/// 启动服务进程并登记为到 `name` 的连接，返回连接目标。
fn connect(name: &str) -> Target {
    let target = Target::new(name).unwrap();
    let server = Command::new(env!("CARGO_BIN_EXE_winc-server"));
    remote::register(target.clone(), Connection::spawn(server).unwrap());
    target
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-server-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 临时目录可能经过符号链接，取真实路径作为远端路径。
    dir.canonicalize().unwrap()
}

/// 本机路径 `path` 在连接 `target` 上对应的虚拟路径。
fn through(target: &Target, path: &Path) -> PathBuf {
    virtual_path(target, path.as_os_str().as_bytes())
}

#[test]
fn files_read_through_a_connection_match_the_local_ones() {
    let target = connect("read-files");
    let dir = scratch("read");
    std::fs::write(dir.join("small.txt"), "héllo\n").unwrap();
    // 超过一次读取的上限（1 MiB），分多次取回；内容逐字节不同，错位能被发现。
    let large: Vec<u8> = (0..3_500_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.join("large.bin"), &large).unwrap();
    std::fs::write(dir.join("empty"), "").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::os::unix::fs::symlink("small.txt", dir.join("link")).unwrap();
    // 不是有效 UTF-8 的文件名：macOS 的文件系统不接受，只在其他系统上检查。
    let odd = std::ffi::OsStr::from_bytes(b"caf\xE9.txt");
    let odd_names = !cfg!(target_os = "macos");
    if odd_names {
        std::fs::write(dir.join(odd), "x").unwrap();
    }
    let far = through(&target, &dir);

    for name in ["small.txt", "large.bin", "empty", "sub", "link"] {
        let (local, remote) = (dir.join(name), far.join(name));
        assert_eq!(
            fs::metadata(&remote).unwrap(),
            fs::metadata(&local).unwrap()
        );
        assert_eq!(
            fs::symlink_metadata(&remote).unwrap(),
            fs::symlink_metadata(&local).unwrap(),
            "{name}"
        );
    }
    assert_eq!(
        fs::read(far.join("small.txt")).unwrap(),
        "héllo\n".as_bytes()
    );
    assert_eq!(fs::read_to_string(far.join("link")).unwrap(), "héllo\n");
    assert_eq!(fs::read(far.join("large.bin")).unwrap(), large);
    assert!(fs::read(far.join("empty")).unwrap().is_empty());
    assert!(fs::is_dir(far.join("sub")) && fs::is_file(far.join("link")));
    assert!(fs::is_symlink(far.join("link")) && fs::exists(&far));

    // 列目录：名字、种类与本机一致，路径仍是该连接上的虚拟路径；非 UTF-8 的名字原样往返。
    let list = |dir: &Path| {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .map(|entry| (entry.name.clone(), entry.kind, entry.path()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    };
    let (near, remote) = (list(&dir), list(&far));
    assert_eq!(remote.len(), 5 + usize::from(odd_names));
    for ((name, kind, local), (far_name, far_kind, far_path)) in near.iter().zip(&remote) {
        assert_eq!((name, kind), (far_name, far_kind));
        assert_eq!(*far_path, through(&target, local));
        assert_eq!(
            winc_backend::locate(far_path).unwrap().path,
            local.as_os_str().as_bytes()
        );
    }
    if odd_names {
        assert_eq!(fs::read(far.join(odd)).unwrap(), b"x");
    }

    remote::disconnect(&target);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn remote_errors_keep_their_kind() {
    let target = connect("errors");
    let dir = scratch("errors");
    std::fs::write(dir.join("file"), "x").unwrap();
    let far = through(&target, &dir);

    let missing = fs::metadata(far.join("missing")).unwrap_err();
    assert!(fs::is_absent(&missing), "{missing}");
    assert!(fs::is_absent(
        &fs::metadata(far.join("file/below")).unwrap_err()
    ));
    assert!(fs::is_absent(&fs::read(far.join("missing")).unwrap_err()));
    assert!(fs::read_dir(far.join("file")).is_err());
    assert!(fs::read(&far).is_err(), "目录不能当作文件读取");
    // 非 UTF-8 的内容按文本读取时是数据错误，不是连接错误。
    std::fs::write(dir.join("binary"), [0xFF, 0xFE]).unwrap();
    let error = fs::read_to_string(far.join("binary")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // 出错之后连接仍然可用。
    assert_eq!(fs::read(far.join("file")).unwrap(), b"x");

    remote::disconnect(&target);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn connections_are_per_target_and_refuse_threads_that_must_not_wait() {
    let target = connect("per-target");
    let dir = scratch("per-target");
    std::fs::write(dir.join("file"), "x").unwrap();
    let file = through(&target, &dir.join("file"));
    assert!(fs::is_file(&file));

    // 另一个目标没有连接；同一路径在不同目标下互不相通。
    let other = through(&Target::new("someone-else").unwrap(), &dir.join("file"));
    let error = fs::metadata(&other).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);

    // 不允许等待的线程：立即返回，不发出请求；本机路径不受影响。
    let (far, near) = (file.clone(), dir.join("file"));
    let kinds = std::thread::spawn(move || {
        remote::forbid_blocking();
        (
            fs::metadata(&far).unwrap_err().kind(),
            fs::read(&far).unwrap_err().kind(),
            fs::is_file(&far),
            fs::is_file(&near),
        )
    });
    assert_eq!(
        kinds.join().unwrap(),
        (
            io::ErrorKind::WouldBlock,
            io::ErrorKind::WouldBlock,
            false,
            true
        )
    );

    // 断开之后不再可用。
    remote::disconnect(&target);
    let error = fs::metadata(&file).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_are_created_changed_and_removed_through_a_connection() {
    let target = connect("change");
    let dir = scratch("change");
    let far = through(&target, &dir);

    fs::create_dir_all(far.join("a/b")).unwrap();
    assert!(fs::create_dir(far.join("x/y")).is_err(), "上级目录不存在");
    fs::create_dir(far.join("a/c")).unwrap();
    assert!(dir.join("a/b").is_dir() && dir.join("a/c").is_dir());

    let file = far.join("a/b/note.txt");
    fs::create_new(&file, b"one").unwrap();
    let error = fs::create_new(&file, b"two").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(dir.join("a/b/note.txt")).unwrap(), b"one");
    fs::write(&file, "three").unwrap();

    let moved = far.join("a/c/moved.txt");
    fs::rename(&file, &moved).unwrap();
    assert!(!fs::exists(&file) && fs::is_file(&moved));
    assert_eq!(fs::copy(&moved, &file).unwrap(), 5);
    assert_eq!(std::fs::read(dir.join("a/b/note.txt")).unwrap(), b"three");

    std::os::unix::fs::symlink("note.txt", dir.join("a/b/link")).unwrap();
    assert_eq!(
        fs::read_link(far.join("a/b/link")).unwrap(),
        Path::new("note.txt")
    );
    fs::copy_symlink(far.join("a/b/link"), far.join("a/b/copy")).unwrap();
    assert_eq!(
        std::fs::read_link(dir.join("a/b/copy")).unwrap(),
        Path::new("note.txt")
    );
    fs::remove_file(&moved).unwrap();
    assert!(!dir.join("a/c/moved.txt").exists());
    assert!(fs::is_absent(&fs::remove_file(&moved).unwrap_err()));

    // 删除目录连同其中的内容；符号链接只删除链接本身，不进入它指向的目录。
    std::fs::create_dir_all(dir.join("kept/inner")).unwrap();
    std::fs::write(dir.join("kept/inner/file"), "x").unwrap();
    std::fs::create_dir_all(dir.join("gone/deep")).unwrap();
    std::fs::write(dir.join("gone/deep/file"), "x").unwrap();
    std::os::unix::fs::symlink(dir.join("kept"), dir.join("gone/link")).unwrap();
    fs::remove_all(far.join("gone/link")).unwrap();
    assert!(dir.join("kept/inner/file").is_file() && dir.join("gone/deep").is_dir());
    fs::remove_all(far.join("gone")).unwrap();
    assert!(!dir.join("gone").exists() && dir.join("kept/inner/file").is_file());
    assert!(fs::is_absent(
        &fs::remove_all(far.join("gone")).unwrap_err()
    ));

    remote::disconnect(&target);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_are_copied_between_this_machine_and_remote_hosts() {
    use std::os::unix::fs::PermissionsExt;

    let (near, dir) = (scratch("transfer-near"), scratch("transfer-far"));
    let (one, two) = (connect("transfer-one"), connect("transfer-two"));
    let (far, other) = (through(&one, &dir), through(&two, &dir));
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;

    // 本机到远端：内容和可执行位都带过去。
    std::fs::write(near.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(near.join("run.sh"), PermissionsExt::from_mode(0o750)).unwrap();
    assert_eq!(
        fs::copy(near.join("run.sh"), far.join("run.sh")).unwrap(),
        18
    );
    assert_eq!(
        std::fs::read(dir.join("run.sh")).unwrap(),
        b"#!/bin/sh\necho hi\n"
    );
    assert_eq!(mode(&dir.join("run.sh")), 0o750);
    // 远端到本机，以及两台远端之间。
    std::fs::write(dir.join("data"), "from afar").unwrap();
    assert_eq!(fs::copy(far.join("data"), near.join("data")).unwrap(), 9);
    assert_eq!(std::fs::read(near.join("data")).unwrap(), b"from afar");
    assert_eq!(fs::copy(far.join("data"), other.join("again")).unwrap(), 9);
    assert_eq!(std::fs::read(dir.join("again")).unwrap(), b"from afar");
    // 符号链接按原样的目标带过去。
    std::os::unix::fs::symlink("run.sh", near.join("link")).unwrap();
    fs::copy_symlink(near.join("link"), far.join("link")).unwrap();
    assert_eq!(
        std::fs::read_link(dir.join("link")).unwrap(),
        Path::new("run.sh")
    );
    fs::copy_symlink(far.join("link"), near.join("back")).unwrap();
    assert_eq!(
        std::fs::read_link(near.join("back")).unwrap(),
        Path::new("run.sh")
    );
    // 源文件不存在：目标不出现。
    assert!(fs::is_absent(
        &fs::copy(near.join("none"), far.join("none")).unwrap_err()
    ));
    assert!(!dir.join("none").exists());

    // 比留在内存里的上限更大的内容转存到磁盘再写入，不留下临时文件；读回来一致。
    let large: Vec<u8> = (0..66_000_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(near.join("large.bin"), &large).unwrap();
    let copied = fs::copy(near.join("large.bin"), far.join("large.bin")).unwrap();
    assert_eq!(copied, large.len() as u64);
    assert!(std::fs::read(dir.join("large.bin")).unwrap() == large);
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(
        !names
            .iter()
            .any(|name| name.to_string_lossy().starts_with(".winc-")),
        "{names:?}"
    );
    std::fs::remove_file(near.join("large.bin")).unwrap();

    // 传输中途停止：两个方向上目标都不出现，对端不留下临时文件。
    let big = vec![7u8; 3 << 20];
    std::fs::write(near.join("big.bin"), &big).unwrap();
    std::fs::write(dir.join("far-big.bin"), &big).unwrap();
    for (from, to, seen) in [
        (near.join("big.bin"), far.join("stopped.bin"), dir.clone()),
        (
            far.join("far-big.bin"),
            near.join("stopped.bin"),
            near.clone(),
        ),
    ] {
        let reported = std::cell::Cell::new(0u64);
        let stop = |bytes: u64| {
            reported.set(reported.get() + bytes);
            false
        };
        let error = fs::copy_watched(&from, &to, &stop).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert!(reported.get() > 0 && reported.get() < big.len() as u64);
        let names: Vec<_> = std::fs::read_dir(&seen)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            !names.iter().any(|name| {
                let name = name.to_string_lossy();
                name.contains("stopped") || name.starts_with(".winc-")
            }),
            "{names:?}"
        );
    }
    // 不停止时报告的字节数合计等于文件大小。
    let total = std::cell::Cell::new(0u64);
    let count = |bytes: u64| {
        total.set(total.get() + bytes);
        true
    };
    fs::copy_watched(far.join("far-big.bin"), near.join("whole.bin"), &count).unwrap();
    assert_eq!(total.get(), big.len() as u64);
    let copied = fs::copy(far.join("large.bin"), near.join("large.bin")).unwrap();
    assert_eq!(copied, large.len() as u64);
    assert!(std::fs::read(near.join("large.bin")).unwrap() == large);
    remote::disconnect(&one);
    remote::disconnect(&two);
}

#[test]
fn a_command_can_ask_the_user_through_the_connection() {
    let target = connect("asking");
    let root = virtual_path(&target, b"/");
    let script = r#"a=$("$GIT_ASKPASS" 'Password for x: ') && printf 'got:%s' "$a"
SSH_ASKPASS_PROMPT=confirm "$SSH_ASKPASS" 'Trust it?' || printf ' declined:%s' "$?""#;
    let asked = std::sync::Mutex::new(Vec::new());
    let answer = |ask: &remote::Ask| {
        asked.lock().unwrap().push(ask.clone());
        ask.mode.is_empty().then(|| b"s3cret".to_vec())
    };
    let exec = sh(script).asking();
    let output = remote::exec_asking(&root, &exec, &|| false, &answer).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "got:s3cret declined:1",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ask = |mode: &str, prompt: &str| remote::Ask {
        mode: mode.into(),
        prompt: prompt.into(),
    };
    assert_eq!(
        *asked.lock().unwrap(),
        [ask("", "Password for x: "), ask("confirm", "Trust it?")]
    );
    // 没有要求转交输入请求的命令：环境里没有 askpass 程序。
    let plain = run(&root, &sh(r#"printf '%s' "${GIT_ASKPASS:-none}""#)).unwrap();
    assert_eq!(plain.stdout, b"none");
    // 提问用的套接字随命令结束而删除。
    let left: Vec<_> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("winc-ask-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    remote::disconnect(&target);
}

#[test]
fn documents_are_opened_and_saved_through_a_connection() {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    let target = connect("save");
    let dir = scratch("save");
    let far = through(&target, &dir);
    let options = fs::SaveOptions {
        in_memory: true,
        ..Default::default()
    };

    // 新文件，所在的目录一并创建；返回的是写入后的文件信息。
    let file = far.join("sub/note.txt");
    let written = fs::save(&file, &options, |out| out.write_all(b"one")).unwrap();
    assert_eq!(std::fs::read(dir.join("sub/note.txt")).unwrap(), b"one");
    assert_eq!(written, fs::metadata(dir.join("sub/note.txt")).unwrap());

    // 已有文件的权限保留；内容分多段上传（3 MB），目录中不留临时文件。
    let local = dir.join("sub/note.txt");
    std::fs::set_permissions(&local, std::fs::Permissions::from_mode(0o750)).unwrap();
    let large: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let bom = fs::SaveOptions {
        bom: true,
        ..options
    };
    let written = fs::save(&file, &bom, |out| {
        large
            .chunks(70_000)
            .try_for_each(|chunk| out.write_all(chunk))
    })
    .unwrap();
    let on_disk = std::fs::read(&local).unwrap();
    assert_eq!(on_disk[..3], [0xEF, 0xBB, 0xBF]);
    assert!(on_disk[3..] == large[..]);
    assert_eq!(written.unix.unwrap().mode & 0o777, 0o750);
    assert_eq!(std::fs::read_dir(dir.join("sub")).unwrap().count(), 1);

    // 打开：文件信息、开头的一段、可重复的顺序读取，以及按偏移读取的句柄。
    let mut opened = fs::open(&file, 5).unwrap();
    assert_eq!(opened.metadata, fs::metadata(&local).unwrap());
    assert_eq!(opened.head, on_disk[..5]);
    for _ in 0..2 {
        let mut all = Vec::new();
        opened.reader().unwrap().read_to_end(&mut all).unwrap();
        assert!(all == on_disk);
    }
    let handle = opened.into_handle();
    let mut buf = [0u8; 7];
    assert_eq!(handle.read_full(1_500_000, &mut buf).unwrap(), 7);
    assert_eq!(buf, on_disk[1_500_000..1_500_007]);
    assert_eq!(
        handle
            .read_full(on_disk.len() as u64 - 3, &mut buf)
            .unwrap(),
        3
    );
    assert_eq!(
        handle.try_clone().unwrap().metadata().unwrap().len,
        on_disk.len() as u64
    );
    assert!(
        fs::open(far.join("sub"), 4).is_err(),
        "目录不能当作文件打开"
    );

    // 本机一侧写内容失败：放弃保存，文件不变，错误原样返回。
    let error = fs::save(&file, &options, |out| {
        out.write_all(b"partial")?;
        Err(io::Error::other("cannot encode"))
    })
    .unwrap_err();
    assert!(matches!(&error, fs::SaveError::Io(e) if e.to_string() == "cannot encode"));
    assert!(std::fs::read(&local).unwrap() == on_disk);

    // 只读的文件、有多个硬链接而内容无法原地写入：对端的判断按类型传回。
    std::fs::set_permissions(&local, std::fs::Permissions::from_mode(0o444)).unwrap();
    let error = fs::save(&file, &options, |out| out.write_all(b"x")).unwrap_err();
    assert!(matches!(error, fs::SaveError::ReadOnly), "{error}");
    std::fs::set_permissions(&local, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::hard_link(&local, dir.join("sub/other")).unwrap();
    let pieces = fs::SaveOptions::default();
    let error = fs::save(&file, &pieces, |out| out.write_all(b"x")).unwrap_err();
    assert!(
        matches!(error, fs::SaveError::IdentityWouldChange),
        "{error}"
    );
    fs::save(&file, &options, |out| out.write_all(b"both")).unwrap();
    assert_eq!(std::fs::read(dir.join("sub/other")).unwrap(), b"both");

    remote::disconnect(&target);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 在 `dir` 所在的主机上运行命令，不取消。
fn run(dir: &Path, exec: &remote::Exec) -> io::Result<remote::Output> {
    remote::exec(dir, exec, &|| false)
}

fn sh(script: &str) -> remote::Exec {
    remote::Exec::new("sh").arg("-c").arg(script)
}

#[test]
fn commands_run_on_the_remote_host_and_report_their_output() {
    let target = connect("exec");
    let dir = scratch("exec");
    let far = through(&target, &dir);

    // 两路输出各自收集，退出码原样返回；非零的退出码不是错误。
    let output = run(&far, &sh("printf out; printf err >&2; exit 3")).unwrap();
    assert_eq!(
        (output.code, output.stdout, output.stderr),
        (Some(3), b"out".to_vec(), b"err".to_vec())
    );
    // 工作目录、环境变量和标准输入。
    let output = run(&far, &remote::Exec::new("pwd")).unwrap();
    assert_eq!(output.stdout, [dir.as_os_str().as_bytes(), b"\n"].concat());
    let exec = sh("printf '%s|' \"$WINC_TEST_VALUE\"; cat")
        .env("WINC_TEST_VALUE", "值 with spaces")
        .stdin("from stdin");
    let output = run(&far, &exec).unwrap();
    assert_eq!(output.stdout, "值 with spaces|from stdin".as_bytes());
    // 参数原样传递，不经过 shell 解释。
    let exec = remote::Exec::new("printf").arg("%s").arg("$HOME; `id` *");
    assert_eq!(run(&far, &exec).unwrap().stdout, b"$HOME; `id` *");
    // 超过一帧的输出分段取回：3 MB，内容逐行不同。
    let output = run(&far, &sh("seq 1 400000")).unwrap();
    assert_eq!(output.code, Some(0));
    assert!(output.stdout.len() > 2_500_000);
    assert!(output.stdout.starts_with(b"1\n2\n") && output.stdout.ends_with(b"\n400000\n"));
    // 被信号结束的命令没有退出码。
    assert_eq!(run(&far, &sh("kill -9 $$")).unwrap().code, None);

    // 无法启动：对端报告的错误种类原样返回；工作目录不存在同理。
    let error = run(&far, &remote::Exec::new("winc-no-such-program")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(run(&far.join("missing"), &remote::Exec::new("pwd")).is_err());
    // 本机路径不经过这里。
    let error = run(&dir, &remote::Exec::new("pwd")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

    remote::disconnect(&target);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_running_command_can_be_cancelled_and_does_not_block_other_requests() {
    let target = connect("cancel");
    let dir = scratch("cancel");
    std::fs::write(dir.join("file"), "x").unwrap();
    let far = through(&target, &dir);

    // 命令还启动了一个后台的子进程：取消时两者一起结束，否则子进程占着输出，等待不会结束。
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let running = {
        let (far, cancel) = (far.clone(), Arc::clone(&cancel));
        std::thread::spawn(move || {
            let cancelled = || cancel.load(Ordering::Relaxed);
            remote::exec(&far, &sh("sleep 30 & sleep 30"), &cancelled)
        })
    };
    // 命令运行期间，同一连接上的文件操作照常应答。
    std::thread::sleep(Duration::from_millis(100));
    let asked = Instant::now();
    assert_eq!(fs::read(far.join("file")).unwrap(), b"x");
    assert!(asked.elapsed() < Duration::from_secs(2));

    cancel.store(true, Ordering::Relaxed);
    let error = running.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(started.elapsed() < Duration::from_secs(10));
    // 取消之后连接仍然可用。
    assert_eq!(run(&far, &sh("printf ok")).unwrap().stdout, b"ok");

    // 连接断开时，等待中的命令以连接中断结束，远端的命令不会留下。时长取一个不会与其他
    // 进程相同的值，用来在进程表中辨认。
    let waiting = {
        let far = far.clone();
        std::thread::spawn(move || run(&far, &sh("sleep 30471")))
    };
    std::thread::sleep(Duration::from_millis(100));
    let left = || {
        let found = Command::new("pgrep").args(["-f", "sleep 30471"]).output();
        found.is_ok_and(|output| output.status.success())
    };
    let listed = left();
    remote::disconnect(&target);
    let error = waiting.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    // 没有 `pgrep` 的系统上无法查看进程表，只检查上面的结果。
    if listed {
        let deadline = Instant::now() + Duration::from_secs(5);
        while left() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!left(), "连接断开后远端的命令仍在运行");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 远端仓库的状态：`git` 在远端运行，输出传回本机。本机没有安装 `git` 时跳过。
#[test]
fn git_runs_in_a_remote_repository() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let target = connect("git");
    let dir = scratch("git");
    let far = through(&target, &dir);
    let git = |args: &[&str]| {
        let mut exec = remote::Exec::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null");
        for arg in args {
            exec = exec.arg(arg);
        }
        run(&far, &exec).unwrap()
    };
    assert_eq!(git(&["init", "-q", "-b", "main"]).code, Some(0));
    std::fs::write(dir.join("new file.txt"), "x").unwrap();
    let status = git(&["status", "--porcelain=v1", "-z", "--branch"]);
    assert_eq!(
        status.code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(
        status.stdout,
        b"## No commits yet on main\0?? new file.txt\0"
    );
    // 仓库的标记经文件操作同样可见。
    assert!(fs::is_dir(far.join(".git")));

    remote::disconnect(&target);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 本机的系统和架构，写法与引导脚本报告的相同。
fn system() -> String {
    let uname = |flag| {
        let output = Command::new("uname").arg(flag).output().unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    format!("{}-{}", uname("-s"), uname("-m"))
}

/// 把刚构建的服务进程作为本机系统的程序，上传目录为 `dir`。
fn deployment(dir: &str, digest: &str) -> remote::Deploy {
    remote::Deploy {
        dir: dir.to_owned(),
        programs: vec![remote::Program {
            system: system(),
            digest: digest.to_owned(),
            content: std::fs::read(env!("CARGO_BIN_EXE_winc-server")).unwrap(),
        }],
    }
}

/// 在本机直接运行引导命令，不经过 `ssh`。
fn bootstrap(deploy: &remote::Deploy) -> Command {
    let words = deploy.command().unwrap();
    let mut command = Command::new(&words[0]);
    command.args(&words[1..]);
    command
}

#[test]
fn the_server_is_uploaded_once_and_started_from_the_upload_directory() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("deploy");
    let home = dir.join("it's home");
    let deploy = deployment(home.to_str().unwrap(), "0af3");
    let installed = home.join("0af3/winc-server");
    std::fs::write(dir.join("note.txt"), "here").unwrap();

    // 第一次连接：上传目录和程序都不存在，上传后启动，连接可用。
    let target = Target::new("deploy-first").unwrap();
    let connection =
        Connection::deploy(bootstrap(&deploy), &deploy, &remote::Abort::default()).unwrap();
    remote::register(target.clone(), connection);
    assert_eq!(
        fs::read(through(&target, &dir.join("note.txt"))).unwrap(),
        b"here"
    );
    assert!(std::fs::read(&installed).unwrap() == deploy.programs[0].content);
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!((mode(&installed), mode(&home)), (0o700, 0o700));
    // 目录中只有程序本身，没有留下临时文件。
    assert_eq!(std::fs::read_dir(home.join("0af3")).unwrap().count(), 1);
    remote::disconnect(&target);

    // 上传目录中的其他内容：旧版本的程序、带有其他文件的版本目录、名字不是摘要的目录和文件。
    for old in ["aaaa", "bbbb", "notes"] {
        std::fs::create_dir(home.join(old)).unwrap();
        std::fs::write(home.join(old).join("winc-server"), "old").unwrap();
    }
    std::fs::write(home.join("bbbb/kept by someone"), "").unwrap();
    std::fs::write(home.join("readme"), "").unwrap();

    // 第二次连接：程序已在，不再上传。这里给出的内容长度相同但不是程序，若被上传则无法启动。
    let mut reused = deploy.clone();
    reused.programs[0].content = vec![0; deploy.programs[0].content.len()];
    let connection =
        Connection::deploy(bootstrap(&reused), &reused, &remote::Abort::default()).unwrap();
    assert!(std::fs::read(&installed).unwrap() == deploy.programs[0].content);
    drop(connection);
    // 已有的程序残缺（字节数不符）：重新上传后照常启动。
    std::fs::write(&installed, "damaged").unwrap();
    std::fs::set_permissions(&installed, PermissionsExt::from_mode(0o700)).unwrap();
    drop(Connection::deploy(bootstrap(&deploy), &deploy, &remote::Abort::default()).unwrap());
    assert!(std::fs::read(&installed).unwrap() == deploy.programs[0].content);
    // 连接时只清理旧版本：名字是摘要、其中只有程序本身的目录。其余的都不动。
    let mut left: Vec<_> = (std::fs::read_dir(&home).unwrap())
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["0af3", "bbbb", "notes", "readme"]);
    assert!(home.join("bbbb/winc-server").is_file() && home.join("notes/winc-server").is_file());

    // 以 `~/` 开头的上传目录相对于主目录。
    let relative = deployment("~/servers", "0af3");
    let mut command = bootstrap(&relative);
    command.env("HOME", &dir);
    drop(Connection::deploy(command, &relative, &remote::Abort::default()).unwrap());
    assert!(dir.join("servers/0af3/winc-server").is_file());
}

#[test]
fn a_connection_that_cannot_start_the_server_reports_why() {
    let dir = scratch("deploy-errors");

    // 远端的系统没有对应的程序：报告远端的系统名，不上传任何内容。
    let mut other = deployment(dir.join("other").to_str().unwrap(), "0af3");
    other.programs[0].system = "Plan9-mips".to_owned();
    let error =
        Connection::deploy(bootstrap(&other), &other, &remote::Abort::default()).unwrap_err();
    assert!(
        matches!(&error, remote::ConnectError::Unsupported(found) if *found == system()),
        "{error}"
    );
    assert!(!dir.join("other").exists());

    // 上传目录无法创建：原因取自错误输出。
    std::fs::write(dir.join("file"), "").unwrap();
    let blocked = deployment(dir.join("file/under").to_str().unwrap(), "0af3");
    let error =
        Connection::deploy(bootstrap(&blocked), &blocked, &remote::Abort::default()).unwrap_err();
    assert!(
        matches!(&error, remote::ConnectError::Failed(why) if why.contains("mkdir")),
        "{error}"
    );

    // 上传的内容不是可以运行的程序：启动失败，而不是停在等待应答上；原因取自错误输出。
    let mut broken = deployment(dir.join("broken").to_str().unwrap(), "0af3");
    broken.programs[0].content = b"not a program".to_vec();
    let error = Connection::deploy(bootstrap(&broken), &broken, &remote::Abort::default());
    let error = error.unwrap_err();
    assert!(
        matches!(&error, remote::ConnectError::Failed(why) if why.contains("winc-server")),
        "{error}"
    );

    // 连接卡在引导阶段（这里是一条不结束的命令）：中止后立即返回，子进程被结束。
    let abort = remote::Abort::default();
    let mut stuck = Command::new("sh");
    stuck.args(["-c", "sleep 30472"]);
    let stopper = abort.clone();
    let waiter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        stopper.abort();
    });
    let started = Instant::now();
    let error = Connection::deploy(stuck, &blocked, &abort).unwrap_err();
    assert!(matches!(error, remote::ConnectError::Aborted), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10));
    waiter.join().unwrap();
    // 开始之前就已中止的连接不会启动引导。
    let error = Connection::deploy(bootstrap(&blocked), &blocked, &abort).unwrap_err();
    assert!(matches!(error, remote::ConnectError::Aborted), "{error}");
    // 服务进程报告启动之后没有应答（这里是一条不结束的命令）：中止同样立即返回。
    let abort = remote::Abort::default();
    let mut silent = Command::new("sh");
    silent.args(["-c", "echo winc:ready; sleep 30481"]);
    let stopper = abort.clone();
    let waiter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        stopper.abort();
    });
    let started = Instant::now();
    let error = Connection::deploy(silent, &blocked, &abort).unwrap_err();
    assert!(matches!(error, remote::ConnectError::Aborted), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10));
    waiter.join().unwrap();
    // 连接建立之后中止：连接断开，其上的请求随即失败，不算意外中断。
    let abort = remote::Abort::default();
    let late = deployment(dir.join("late").to_str().unwrap(), "0af3");
    let connection = Connection::deploy(bootstrap(&late), &late, &abort).unwrap();
    let (sender, closed) = std::sync::mpsc::channel();
    connection.on_close(move || {
        let _ = sender.send(());
    });
    let target = Target::new("deploy-late").unwrap();
    remote::register(target.clone(), connection);
    assert!(remote::is_connected(&target));
    abort.abort();
    assert!(!remote::is_connected(&target));
    assert!(fs::metadata(winc_backend::virtual_path(&target, b"/")).is_err());
    assert!(closed.recv_timeout(Duration::from_millis(300)).is_err());
    remote::disconnect(&target);

    // 子进程没有运行引导命令就结束了（例如登录失败）：给出它的错误输出。
    let mut refused = Command::new("sh");
    refused.args(["-c", "echo Permission denied >&2; exit 255"]);
    let error = Connection::deploy(refused, &blocked, &remote::Abort::default()).unwrap_err();
    assert!(
        matches!(&error, remote::ConnectError::Failed(why) if why == "Permission denied"),
        "{error}"
    );
}

#[test]
fn a_connection_that_ends_unexpectedly_is_reported_once() {
    let target = Target::new("ended").unwrap();
    let server = Command::new(env!("CARGO_BIN_EXE_winc-server"));
    let connection = Connection::spawn(server).unwrap();
    let (sender, ended) = std::sync::mpsc::channel();
    connection.on_close(move || sender.send(()).unwrap());
    remote::register(target.clone(), connection);
    assert!(remote::is_connected(&target));

    // 对端被强制结束（命令的父进程就是服务进程）：通知一次，此后该目标不再算已连接，请求立即失败。
    let killed = run(&virtual_path(&target, b"/"), &sh("kill -9 $PPID")).unwrap_err();
    assert_eq!(killed.kind(), io::ErrorKind::BrokenPipe);
    ended.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!remote::is_connected(&target));
    let error = fs::metadata(virtual_path(&target, b"/")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);

    // 本程序主动断开不算意外中断：不通知。丢弃连接时等到读取应答的线程结束。
    let server = Command::new(env!("CARGO_BIN_EXE_winc-server"));
    let connection = Connection::spawn(server).unwrap();
    let (sender, ended) = std::sync::mpsc::channel::<()>();
    connection.on_close(move || sender.send(()).unwrap());
    drop(connection);
    assert!(ended.try_recv().is_err());
}

#[test]
fn the_server_watches_a_folder_and_reports_changes() {
    use std::io::{BufRead, BufReader};

    let target = connect("watch");
    let dir = scratch("watch");
    std::fs::create_dir(dir.join("sub")).unwrap();
    let far = through(&target, &dir);

    // 服务进程以 `watch <目录>` 再次运行自身；先报告开始监视，再逐条报告发生变化的路径。
    let watch = remote::Exec::this_server()
        .arg("watch")
        .arg(dir.as_os_str().as_bytes());
    let running = remote::spawn(&far, &watch).unwrap();
    let stopper = running.stopper();
    let mut messages = BufReader::new(running).split(0).map(Result::unwrap);
    assert_eq!(messages.next().unwrap(), b"S");
    std::fs::write(dir.join("sub/new.txt"), "x").unwrap();
    let changed = dir.join("sub/new.txt");
    let reported = messages
        .by_ref()
        .any(|message| message.strip_prefix(b"P") == Some(changed.as_os_str().as_bytes()));
    assert!(reported);

    // 不再需要时结束它：输出随之结束，远端不留下监视的进程。
    stopper.stop();
    assert!(messages.all(|_| true));
    let gone = Instant::now() + Duration::from_secs(10);
    let left = || {
        let found = Command::new("pgrep")
            .args(["-f", "winc-server watch"])
            .output();
        found.is_ok_and(|found| !found.stdout.is_empty())
    };
    while left() && Instant::now() < gone {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!left());
    remote::disconnect(&target);
}

#[test]
fn common_operations_take_one_round_trip() {
    use std::io::Read;

    let target = connect("trips");
    let dir = scratch("trips");
    let far = through(&target, &dir);
    std::fs::write(dir.join("small.txt"), "hello").unwrap();
    std::fs::write(dir.join("empty"), "").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    let trips = |work: &mut dyn FnMut()| {
        let before = remote::requests(&target);
        work();
        remote::requests(&target) - before
    };

    // 打开并读完一个小文件：文件信息、开头和全部内容在一次往返中取回，再读一遍也不发请求。
    let mut content = Vec::new();
    let opened = trips(&mut || {
        let mut file = fs::open(far.join("small.txt"), 3).unwrap();
        assert_eq!((file.head.as_slice(), file.metadata.len), (&b"hel"[..], 5));
        for _ in 0..2 {
            content.clear();
            file.reader().unwrap().read_to_end(&mut content).unwrap();
        }
    });
    assert_eq!((opened, content.as_slice()), (1, &b"hello"[..]));
    let empty = trips(&mut || {
        let mut file = fs::open(far.join("empty"), 8).unwrap();
        let mut content = Vec::new();
        file.reader().unwrap().read_to_end(&mut content).unwrap();
        assert!(file.head.is_empty() && content.is_empty());
    });
    assert_eq!(empty, 1);
    // 目录不能打开；不存在的文件保持原来的错误。
    let failure = |name: &str| fs::open(far.join(name), 0).err().unwrap();
    assert_eq!(failure("sub").kind(), io::ErrorKind::IsADirectory);
    assert!(fs::is_absent(&failure("none")));

    // 若干路径的文件信息一次取回，顺序与路径对应，各自的错误互不影响；本机的路径夹在其中。
    let paths = [
        far.join("small.txt"),
        far.join("none"),
        dir.join("small.txt"),
        far.join("sub"),
    ];
    let mut found = Vec::new();
    let batched = trips(&mut || found = fs::metadata_many(&paths));
    assert_eq!(batched, 1);
    assert!(found[0].as_ref().is_ok_and(|m| m.is_file() && m.len == 5));
    assert!(found[1].as_ref().is_err_and(fs::is_absent));
    assert!(found[2].as_ref().is_ok_and(|m| m.len == 5));
    assert!(found[3].as_ref().is_ok_and(fs::Metadata::is_dir));
    // 没有连接的主机：每个路径都是同一种错误。
    let nowhere = virtual_path(&Target::new("trips-nowhere").unwrap(), b"/x");
    let lost = fs::metadata_many(&[nowhere]);
    assert_eq!(
        lost[0].as_ref().unwrap_err().kind(),
        io::ErrorKind::NotConnected
    );
    remote::disconnect(&target);
}

#[test]
fn requests_and_replies_larger_than_one_frame_are_split() {
    use std::os::unix::fs::PermissionsExt;

    let target = connect("large");
    let dir = scratch("large");
    let far = through(&target, &dir);

    // 内容超过一帧的写入和新建：请求分成几段发出。
    let large: Vec<u8> = (0..2_500_000u32).map(|i| (i % 251) as u8).collect();
    fs::write(far.join("written.bin"), &large).unwrap();
    assert!(std::fs::read(dir.join("written.bin")).unwrap() == large);
    fs::create_new(far.join("created.bin"), &large).unwrap();
    assert!(std::fs::read(dir.join("created.bin")).unwrap() == large);
    // 命令的输入同样可以超过一帧。
    let output = run(&far, &sh("wc -c").stdin(large.clone())).unwrap();
    let count = String::from_utf8(output.stdout).unwrap();
    assert_eq!(count.trim(), "2500000");

    // 目录项的名字合计远超一帧（3 MB）：分批取回，一项不少。
    let many = dir.join("many");
    std::fs::create_dir(&many).unwrap();
    let name = |i: usize| format!("{i:05}-{}", "n".repeat(190));
    for i in 0..15_000 {
        std::fs::write(many.join(name(i)), "").unwrap();
    }
    let mut names: Vec<_> = (fs::read_dir(far.join("many")).unwrap())
        .map(|entry| entry.unwrap().name.into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names.len(), 15_000);
    assert!(names.iter().enumerate().all(|(i, found)| *found == name(i)));
    // 读不了的目录仍然是错误，不是空目录。
    std::fs::create_dir(dir.join("locked")).unwrap();
    std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
    // 以 root 运行时权限不起作用，只在读不了时检查。
    if std::fs::read_dir(dir.join("locked")).is_err() {
        let error = fs::read_dir(far.join("locked")).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
    std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o700)).unwrap();
    remote::disconnect(&target);
}

#[test]
fn special_files_and_links_are_handled_like_on_this_machine() {
    let target = connect("special");
    let dir = scratch("special");
    let far = through(&target, &dir);

    // 命名管道：打开它读取会一直等待另一端，这里直接报错，连接照常可用。
    let made = Command::new("mkfifo").arg(dir.join("pipe")).status();
    if made.is_ok_and(|status| status.success()) {
        let error = fs::read(far.join("pipe")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(fs::is_dir(&far));
    }

    // 经符号链接保存：写入链接指向的文件，链接本身保留。
    std::fs::create_dir(dir.join("real")).unwrap();
    std::fs::write(dir.join("real/config"), "old").unwrap();
    std::os::unix::fs::symlink("real/config", dir.join("link")).unwrap();
    let options = fs::SaveOptions {
        in_memory: true,
        ..Default::default()
    };
    fs::save(far.join("link"), &options, |out| out.write_all(b"new")).unwrap();
    assert!(
        std::fs::symlink_metadata(dir.join("link"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(std::fs::read(dir.join("real/config")).unwrap(), b"new");
    remote::disconnect(&target);
}

#[test]
fn a_command_that_leaves_a_child_holding_its_output_still_finishes() {
    let target = connect("orphan");
    let far = virtual_path(&target, b"/");

    // 后台的子进程继承了输出的管道：命令退出后不再等它，连同它一起结束。
    let started = Instant::now();
    let output = run(&far, &sh("sleep 30473 & echo done")).unwrap();
    assert_eq!((output.code, output.stdout), (Some(0), b"done\n".to_vec()));
    assert!(started.elapsed() < Duration::from_secs(10));
    // 结束信号送达到进程消失之间有一小段时间。
    let gone = |pattern: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        let running = || {
            let found = Command::new("pgrep").args(["-f", pattern]).output();
            found.is_ok_and(|found| !found.stdout.is_empty())
        };
        while running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        !running()
    };
    assert!(gone("sleep 30473"));

    // 输出超过上限：命令被结束，不把全部输出收进内存。
    let endless = sh("yes").limit(1 << 20);
    let error = run(&far, &endless).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);

    // 随到随读：先读到前面的输出，命令结束后得到退出码和标准错误。
    use std::io::Read;
    let mut running =
        remote::spawn(&far, &sh("echo one; echo oops >&2; echo two; exit 3")).unwrap();
    let mut text = String::new();
    running.read_to_string(&mut text).unwrap();
    assert_eq!(text, "one\ntwo\n");
    assert_eq!(
        (running.code(), running.stderr()),
        (Some(Some(3)), &b"oops\n"[..])
    );
    // 没有输出时在期限到达后返回；丢弃时结束仍在运行的命令。
    let mut slow = remote::spawn(&far, &sh("sleep 30474")).unwrap();
    slow.read_until(Some(Instant::now() + Duration::from_millis(100)));
    let error = slow.read(&mut [0; 8]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    drop(slow);
    assert!(gone("sleep 30474"));
    remote::disconnect(&target);
}

/// 经真实的 `ssh` 连接：需要环境变量 `WINC_TEST_SSH_TARGET`（免输入即可登录的主机）和
/// `WINC_TEST_SSH_SERVER`（该主机上服务进程的路径）。
#[test]
#[ignore = "需要一台可以免输入登录、已放好服务进程的主机"]
fn over_ssh() {
    let host = std::env::var("WINC_TEST_SSH_TARGET").unwrap();
    let server = std::env::var("WINC_TEST_SSH_SERVER").unwrap();
    let target = Target::new(&host).unwrap();
    let command = remote::ssh_command(&target, &server, BATCH);
    remote::register(target.clone(), Connection::spawn(command).unwrap());

    let root = virtual_path(&target, b"/");
    assert!(fs::is_dir(&root));
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().name)
        .collect();
    assert!(names.iter().any(|name| name == "etc"), "{names:?}");
    let passwd = fs::read_to_string(root.join("etc/passwd")).unwrap();
    assert!(passwd.contains("root:"));
    let absent = fs::metadata(root.join("no/such/file")).unwrap_err();
    assert!(fs::is_absent(&absent));
    // 命令在远端运行：工作目录是远端的路径。
    let output = run(&root.join("etc"), &sh("pwd; printf done >&2")).unwrap();
    assert_eq!(
        (output.code, output.stdout, output.stderr),
        (Some(0), b"/etc\n".to_vec(), b"done".to_vec())
    );
    // 写入经真实的连接往返：多段上传的内容保存后读回一致，再删除。
    let dir = root.join("tmp/winc over ssh");
    let file = dir.join("large.bin");
    let large: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let options = fs::SaveOptions {
        in_memory: true,
        ..Default::default()
    };
    let written = fs::save(&file, &options, |out| out.write_all(&large)).unwrap();
    assert_eq!(written.len, large.len() as u64);
    assert!(fs::read(&file).unwrap() == large);
    fs::remove_file(&file).unwrap();
    assert!(!fs::exists(&file));
    remote::disconnect(&target);

    // 部署：经登录 shell 运行引导命令，上传并启动服务进程；再次连接时直接启动。
    let deploy = deployment("~/.winc-server/it's deployed", "0af3");
    for _ in 0..2 {
        let command = deploy.ssh_command(&target, BATCH).unwrap();
        remote::register(
            target.clone(),
            Connection::deploy(command, &deploy, &remote::Abort::default()).unwrap(),
        );
        let passwd = fs::read_to_string(root.join("etc/passwd")).unwrap();
        assert!(passwd.contains("root:"));
        remote::disconnect(&target);
    }
    // 连接复用：以可复用的方式连接后，另一条 `ssh` 经它开会话，不再登录；断开后控制套接字
    // 不留下，那条 `ssh` 的选项此后只是让它单独登录。
    let sharing = remote::Ssh {
        share: true,
        ..BATCH
    };
    let command = deploy.ssh_command(&target, sharing).unwrap();
    remote::register(
        target.clone(),
        Connection::deploy(command, &deploy, &remote::Abort::default()).unwrap(),
    );
    let options = remote::shared(&target).unwrap();
    let second = |extra: &[&str]| {
        let mut command = Command::new("ssh");
        command.args(&options).args(["-o", "BatchMode=yes"]);
        if let Some(port) = target.port() {
            command.arg("-p").arg(port.to_string());
        }
        let output = command.args(extra).output().unwrap();
        (output.status.success(), output.stdout)
    };
    assert!(second(&["-O", "check", target.host()]).0);
    assert_eq!(
        second(&[target.host(), "--", "echo shared"]),
        (true, b"shared\n".to_vec())
    );
    let sockets = || -> Vec<_> {
        let dir = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".ssh");
        let names = std::fs::read_dir(dir).unwrap().flatten();
        names
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().starts_with("winc-"))
            .collect()
    };
    assert_eq!(sockets().len(), 1);
    remote::disconnect(&target);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !sockets().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(sockets().is_empty(), "{:?}", sockets());
    assert!(!second(&["-O", "check", target.host()]).0);
    assert_eq!(
        second(&[target.host(), "--", "echo alone"]),
        (true, b"alone\n".to_vec())
    );

    // 无法登录的主机：原因是 `ssh` 的错误输出。
    let nowhere = Target::new("winc-no-such-host.invalid").unwrap();
    let command = deploy.ssh_command(&nowhere, BATCH).unwrap();
    let error = Connection::deploy(command, &deploy, &remote::Abort::default()).unwrap_err();
    assert!(
        matches!(&error, remote::ConnectError::Failed(why) if !why.is_empty()),
        "{error}"
    );
}

/// 写入按 `rate`（字节每秒）限速的一端：模拟带宽有限的网络，写入期间占着写的一方。
struct Throttled<W>(W, u64);

impl<W: io::Write> io::Write for Throttled<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        std::thread::sleep(Duration::from_secs_f64(data.len() as f64 / self.1 as f64));
        self.0.write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// 测量：从远端取回大文件期间，同一条连接上别的请求要等多久。应答一侧限速为 8 MiB/s。
#[test]
#[ignore = "测量用，输出等待时间"]
fn measure_other_requests_during_a_large_read() {
    use std::os::unix::net::UnixStream;

    let dir = scratch("measure-read");
    std::fs::write(dir.join("big.bin"), vec![7u8; 24 << 20]).unwrap();
    std::fs::write(dir.join("small"), "x").unwrap();
    let (requests_in, requests_out) = UnixStream::pair().unwrap();
    let (replies_in, replies_out) = UnixStream::pair().unwrap();
    std::thread::spawn(move || remote::serve(requests_in, Throttled(replies_out, 8 << 20)));
    let target = Target::new("measure-read").unwrap();
    remote::register(
        target.clone(),
        Connection::over(replies_in, requests_out).unwrap(),
    );
    let (far, near) = (through(&target, &dir), dir.clone());
    let started = Instant::now();
    let copy = {
        let far = far.clone();
        std::thread::spawn(move || fs::copy(far.join("big.bin"), near.join("back.bin")).unwrap())
    };
    let mut waits = Vec::new();
    while !copy.is_finished() {
        let asked = Instant::now();
        fs::metadata(far.join("small")).unwrap();
        waits.push(asked.elapsed());
        std::thread::sleep(Duration::from_millis(7));
    }
    copy.join().unwrap();
    waits.sort();
    let at = |part: usize| waits[(waits.len() - 1) * part / 100];
    eprintln!(
        "24 MiB at 8 MiB/s: {:?} in all; {} other requests waited median {:?}, p95 {:?}, max {:?}",
        started.elapsed(),
        waits.len(),
        at(50),
        at(95),
        at(100),
    );
    remote::disconnect(&target);
}
