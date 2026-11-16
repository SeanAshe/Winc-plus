//! Windows 环境加载的测试：候选顺序、大小写、无效环境和非交互式进程。

use super::*;
use std::path::PathBuf;

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("winc-pty-python-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn venv(&self, name: &str) {
        let root = self.0.join(name);
        std::fs::create_dir_all(root.join("Scripts")).unwrap();
        std::fs::write(root.join("pyvenv.cfg"), "home = C:\\Python\n").unwrap();
        std::fs::write(root.join("Scripts/python.exe"), []).unwrap();
    }

    fn activate(&self, program: &[u8], args: &[Vec<u8>], env: &mut Env) {
        activate(self.0.to_str().unwrap().as_bytes(), program, args, env);
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn value<'a>(env: &'a Env, name: &[u8]) -> Option<&'a [u8]> {
    env.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_slice())
}

#[test]
fn valid_environments_are_selected_in_order_and_replace_path_case_insensitively() {
    let t = Temp::new("order");
    t.venv(".venv");
    t.venv("venv");
    for name in [Some(".venv"), Some("venv"), None] {
        let mut env = vec![
            (b"Path".to_vec(), b"C:\\Windows".to_vec()),
            (b"PythonHome".to_vec(), b"C:\\Python".to_vec()),
        ];
        let before = env.clone();
        t.activate(br"C:\Windows\System32\CMD.EXE", &[], &mut env);
        if let Some(name) = name {
            let expected = format!("{};C:\\Windows", t.0.join(name).join("Scripts").display());
            assert_eq!(value(&env, b"PATH"), Some(expected.as_bytes()));
            assert_eq!(
                value(&env, b"VIRTUAL_ENV"),
                Some(t.0.join(name).to_str().unwrap().as_bytes())
            );
            assert_eq!(value(&env, b"PYTHONHOME"), None);
            assert_eq!(
                env.iter()
                    .filter(|(key, _)| key.eq_ignore_ascii_case(b"PATH"))
                    .count(),
                1
            );
            std::fs::remove_file(t.0.join(name).join("pyvenv.cfg")).unwrap();
        } else {
            assert_eq!(env, before);
        }
    }
}

#[test]
fn active_environments_and_commands_are_preserved() {
    let t = Temp::new("preserve");
    t.venv(".venv");
    for key in [b"Virtual_Env".as_slice(), b"Conda_Prefix"] {
        let mut env = vec![(key.to_vec(), b"existing".to_vec())];
        let before = env.clone();
        t.activate(b"pwsh.exe", &[], &mut env);
        assert_eq!(env, before);
    }
    for (program, args) in [
        (b"cmd.exe".as_slice(), vec![b"/c".to_vec()]),
        (b"ssh.exe", vec![]),
        (b"wsl.exe", vec![]),
        (b"python.exe", vec![]),
    ] {
        let mut env = Vec::new();
        t.activate(program, &args, &mut env);
        assert!(env.is_empty());
    }
}

#[test]
fn an_environment_without_an_interpreter_is_ignored() {
    let t = Temp::new("broken");
    t.venv(".venv");
    std::fs::remove_file(t.0.join(".venv/Scripts/python.exe")).unwrap();
    let mut env = Vec::new();
    t.activate(b"powershell.exe", &[], &mut env);
    assert!(env.is_empty());
}

#[test]
fn an_environment_the_user_did_not_allow_is_left_alone() {
    let t = Temp::new("declined");
    t.venv(".venv");
    let mut env = vec![
        (b"Path".to_vec(), b"C:\\Windows".to_vec()),
        (b"WINC_PYTHON".to_vec(), b"0".to_vec()),
    ];
    t.activate(br"C:\Windows\System32\cmd.exe", &[], &mut env);
    assert_eq!(value(&env, b"PATH"), Some(&b"C:\\Windows"[..]));
    assert_eq!(value(&env, b"VIRTUAL_ENV"), None);
    // 原生 shell 没有集成脚本删除这个变量，由辅助进程删除。
    assert_eq!(value(&env, b"WINC_PYTHON"), None);
}
