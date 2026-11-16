//! Python 项目内环境的启动测试：配置加载后的解释器选择、环境还原和项目脚本的执行边界。

use super::*;
use std::os::unix::fs::PermissionsExt;

fn project(t: &Temp, name: &str) -> PathBuf {
    let root = t.0.join("项目 with space ' $value");
    let venv = root.join(name);
    std::fs::create_dir_all(venv.join("bin")).unwrap();
    std::fs::write(venv.join("pyvenv.cfg"), "home = /usr/bin\n").unwrap();
    let python = venv.join("bin/python");
    std::fs::write(&python, "#!/bin/sh\nprintf 'project-python\\n'\n").unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    // 自动加载只调整环境；即使激活文件含有副作用也不读取。
    std::fs::write(venv.join("bin/activate"), "echo PROJECT_SCRIPT_EXECUTED\n").unwrap();
    root
}

fn start(shell: &str, t: &Temp, root: &Path, extra: &[(&str, &str)]) -> Option<Shell> {
    let program = find(shell)?;
    let mut client = Client::start();
    client.spawn_shell_in(
        1,
        &program,
        false,
        env(&t.home(), extra),
        &t.0.join("shell"),
        root,
    );
    client.send(ToHelper::Resize {
        tab: 1,
        size: winc_pty::protocol::Size {
            cols: 240,
            rows: 100,
            width: 1920,
            height: 1600,
        },
    });
    Some(Shell {
        client,
        blocks: Vec::new(),
    })
}

#[test]
fn project_environment_is_loaded_after_user_config_and_can_be_deactivated() {
    for shell in ["bash", "zsh", "fish"] {
        let t = Temp::new(&format!("python-{shell}"));
        let root = project(&t, ".venv");
        let startup = match shell {
            "bash" => "home/.bashrc",
            "zsh" => "home/.zshrc",
            _ => "home/.config/fish/config.fish",
        };
        let config = if shell == "fish" {
            "set -gx PATH /usr/bin /bin\ncd /\n"
        } else {
            "export PATH=/usr/bin:/bin\nPS1='python> '\ncd /\n"
        };
        t.write(startup, config);
        let Some(mut sh) = start(shell, &t, &root, &[("PYTHONHOME", "/old/python")]) else {
            continue;
        };
        let block = sh.run(
            "printenv VIRTUAL_ENV; command -v python; python; printenv PYTHONHOME || echo unset",
        );
        let expected = format!(
            "{}\n{}\nproject-python\nunset",
            root.join(".venv").display(),
            root.join(".venv/bin/python").display()
        );
        assert_eq!(sh.output(&block), expected, "{shell}");
        assert!(!sh.client.text().contains("PROJECT_SCRIPT_EXECUTED"));
        let block = sh.run(
            "deactivate; printenv VIRTUAL_ENV || echo unset; printenv PYTHONHOME; printenv PATH",
        );
        assert_eq!(
            sh.output(&block),
            "unset\n/old/python\n/usr/bin:/bin",
            "{shell}"
        );
        let block = sh.run("printenv VIRTUAL_ENV || echo still-unset");
        assert_eq!(sh.output(&block), "still-unset", "{shell}");
    }
}

#[test]
fn python_uses_the_project_environment_and_its_installed_modules() {
    let t = Temp::new("python-real");
    let root = t.0.join("project with space");
    let venv = root.join(".venv");
    let output = match std::process::Command::new("python3")
        .args(["-m", "venv", "--without-pip"])
        .arg(&venv)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("无法创建测试环境：{error}"),
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = std::process::Command::new(venv.join("bin/python"))
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let site = String::from_utf8(output.stdout).unwrap();
    std::fs::write(
        Path::new(site.trim()).join("winc_venv_only.py"),
        "value = 158\n",
    )
    .unwrap();
    for shell in ["bash", "zsh", "fish"] {
        let Some(mut sh) = start(shell, &t, &root, &[]) else {
            continue;
        };
        let block = sh.run(
            "python -c 'import sys,winc_venv_only; print(sys.prefix); print(winc_venv_only.value)'",
        );
        assert_eq!(
            sh.output(&block),
            format!("{}\n158", venv.display()),
            "{shell}"
        );
    }
}

#[test]
fn an_existing_environment_is_not_replaced() {
    for shell in ["bash", "zsh", "fish"] {
        for variable in ["VIRTUAL_ENV", "CONDA_PREFIX"] {
            let t = Temp::new(&format!("python-existing-{shell}-{variable}"));
            let root = project(&t, ".venv");
            if shell == "bash" {
                t.write("home/.bashrc", "set -e\nPS1='existing> '\n");
            }
            let Some(mut sh) = start(shell, &t, &root, &[(variable, "/existing")]) else {
                continue;
            };
            let block = sh.run(&format!("printenv {variable}"));
            assert_eq!(sh.output(&block), "/existing", "{shell}");
            let block = sh.run("command -v python || echo no-python");
            assert!(
                !sh.output(&block).contains(root.to_str().unwrap()),
                "{shell}"
            );
        }
    }
}

#[test]
fn only_valid_project_environments_are_selected_in_order() {
    for shell in ["bash", "zsh", "fish"] {
        let t = Temp::new(&format!("python-order-{shell}"));
        let root = project(&t, ".venv");
        project(&t, "venv");
        for name in [Some(".venv"), Some("venv"), None] {
            let Some(mut sh) = start(shell, &t, &root, &[]) else {
                break;
            };
            let block = sh.run("printenv VIRTUAL_ENV || echo unset");
            let expected = name.map_or_else(
                || "unset".to_owned(),
                |name| root.join(name).display().to_string(),
            );
            assert_eq!(sh.output(&block), expected, "{shell}");
            drop(sh);
            if let Some(name) = name {
                let file = if name == ".venv" {
                    "bin/python"
                } else {
                    "pyvenv.cfg"
                };
                std::fs::remove_file(root.join(name).join(file)).unwrap();
            }
        }
    }
}

#[test]
fn disabling_shell_integration_keeps_the_terminal_environment() {
    let Some(bash) = find("bash") else { return };
    let t = Temp::new("python-no-integration");
    let root = project(&t, ".venv");
    let mut client = Client::start();
    client.send(ToHelper::Spawn {
        tab: 1,
        program: Some(bash.into_bytes()),
        args: vec![b"--noprofile".to_vec(), b"--norc".to_vec()],
        login: false,
        cwd: root.to_str().unwrap().as_bytes().to_vec(),
        env: env(&t.home(), &[]),
        scrollback: 100,
        size: winc_pty::protocol::Size {
            cols: 240,
            rows: 20,
            width: 1920,
            height: 320,
        },
        integration: None,
        banner: Vec::new(),
    });
    client.send(ToHelper::Input {
        tab: 1,
        bytes: b"printenv VIRTUAL_ENV || echo NO_PROJECT_ENV; exit\r".to_vec(),
        rest: 0,
    });
    assert_eq!(
        client.until_exit_showing(|text| text.contains("\nNO_PROJECT_ENV")),
        0
    );
}

#[test]
fn an_environment_the_user_did_not_allow_is_not_loaded() {
    for shell in ["bash", "zsh", "fish"] {
        let t = Temp::new(&format!("python-declined-{shell}"));
        let root = project(&t, ".venv");
        let Some(mut sh) = start(shell, &t, &root, &[("WINC_PYTHON", "0")]) else {
            continue;
        };
        let block = sh.run("printenv VIRTUAL_ENV || echo no-environment");
        assert_eq!(sh.output(&block), "no-environment", "{shell}");
        let block = sh.run("command -v python || echo no-python");
        assert!(
            !sh.output(&block).contains(root.to_str().unwrap()),
            "{shell}"
        );
        // 这个变量只给集成脚本读，不留在 shell 的环境里。
        let block = sh.run("printenv WINC_PYTHON || echo unset");
        assert_eq!(sh.output(&block), "unset", "{shell}");
    }
}
