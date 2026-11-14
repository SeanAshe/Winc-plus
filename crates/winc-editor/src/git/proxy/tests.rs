//! `proxy` 的测试：系统代理设置的解析、换成环境变量、例外列表，以及不覆盖已有的代理变量。

use super::*;

const SCUTIL: &str = "<dictionary> {
  ExceptionsList : <array> {
    0 : *example.com
    1 : *.local
    2 : localhost
    3 : 10.*
    4 : 169.254/16
  }
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7891
  HTTPSProxy : 127.0.0.1
  ProxyAutoConfigEnable : 0
  SOCKSEnable : 0
  SOCKSPort : 7892
  SOCKSProxy : 127.0.0.1
}
";

#[test]
fn the_macos_proxy_settings_become_environment_variables() {
    let proxy = parse_scutil(SCUTIL);
    assert_eq!(proxy.http.as_deref(), Some("127.0.0.1:7890"));
    assert_eq!(proxy.https.as_deref(), Some("127.0.0.1:7891"));
    // 没开着的不算。
    assert_eq!(proxy.socks, None);
    assert_eq!(
        proxy.vars(),
        [
            ("http_proxy", "http://127.0.0.1:7890".to_owned()),
            ("https_proxy", "http://127.0.0.1:7891".to_owned()),
            // 通配符去掉成为后缀；地址前缀和不完整的网段补成完整的网段。
            (
                "no_proxy",
                "example.com,.local,localhost,10.0.0.0/8,169.254.0.0/16".to_owned()
            ),
        ]
    );
}

#[test]
fn no_proxy_enabled_sets_nothing() {
    let off = SCUTIL
        .replace("HTTPEnable : 1", "HTTPEnable : 0")
        .replace("HTTPSEnable : 1", "HTTPSEnable : 0");
    assert!(parse_scutil(&off).vars().is_empty());
    assert!(parse_scutil("<dictionary> {\n}\n").vars().is_empty());
}

#[test]
fn a_socks_only_proxy_is_used_for_everything() {
    let socks = SCUTIL
        .replace("HTTPEnable : 1", "HTTPEnable : 0")
        .replace("HTTPSEnable : 1", "HTTPSEnable : 0")
        .replace("SOCKSEnable : 0", "SOCKSEnable : 1");
    let vars = parse_scutil(&socks).vars();
    assert_eq!(
        vars[0],
        ("all_proxy", "socks5h://127.0.0.1:7892".to_owned())
    );
}

#[test]
fn the_windows_registry_settings_are_read_in_both_forms() {
    let one = "
HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings
    ProxyEnable    REG_DWORD    0x1
    ProxyServer    REG_SZ    127.0.0.1:7890
    ProxyOverride    REG_SZ    localhost;*.corp.example;<local>
";
    let proxy = parse_registry(one);
    assert_eq!(proxy.http.as_deref(), Some("127.0.0.1:7890"));
    assert_eq!(proxy.https.as_deref(), Some("127.0.0.1:7890"));
    let vars = proxy.vars();
    assert_eq!(vars[2], ("no_proxy", "localhost,.corp.example".to_owned()));

    let each = one.replace("127.0.0.1:7890", "http=a:1;https=b:2;socks=c:3");
    let proxy = parse_registry(&each);
    assert_eq!(proxy.http.as_deref(), Some("a:1"));
    assert_eq!(proxy.https.as_deref(), Some("b:2"));
    assert_eq!(proxy.socks.as_deref(), Some("c:3"));

    // 关着时地址还留在注册表里，不能用。
    let off = one.replace("0x1", "0x0");
    assert!(parse_registry(&off).vars().is_empty());
}

#[test]
fn a_proxy_already_in_the_environment_is_left_alone() {
    let mut command = Command::new("git");
    command.env("HTTPS_PROXY", "http://mine:1");
    apply(&mut command);
    let vars: Vec<_> = command.get_envs().collect();
    assert_eq!(vars.len(), 1, "{vars:?}");

    // 关掉跟随以后什么都不设。
    follow_system_proxy(false);
    let mut command = Command::new("git");
    apply(&mut command);
    assert_eq!(command.get_envs().count(), 0);
    follow_system_proxy(true);
}

#[test]
fn address_prefixes_become_networks_and_odd_patterns_are_dropped() {
    let list = |entries: &[&str]| {
        let entries: Vec<String> = entries.iter().map(|&e| e.to_owned()).collect();
        no_proxy(&entries)
    };
    assert_eq!(
        list(&["127.*", "192.168.*", "172.16.*"]),
        "127.0.0.0/8,192.168.0.0/16,172.16.0.0/16"
    );
    assert_eq!(list(&["10.1.2.0/24"]), "10.1.2.0/24");
    // 写不成网段的前缀、Windows 的 `<local>` 略去。
    assert_eq!(list(&["172.2*", "<local>", "*"]), "");
    assert_eq!(list(&["host.example"]), "host.example");
}

#[test]
fn only_online_commands_follow_the_proxy() {
    use std::ffi::OsStr;
    let online = |args: &[&str]| super::online(args.iter().map(OsStr::new));
    assert!(online(&["fetch", "--all"]));
    assert!(online(&["push", "--set-upstream", "origin", "HEAD"]));
    assert!(online(&[
        "-c",
        "protocol.ext.allow=never",
        "clone",
        "--",
        "url",
        "dir"
    ]));
    assert!(!online(&["add", "--all", "--", "a.rs"]));
    assert!(!online(&["commit", "-m", "x"]));
    assert!(!online(&["-c", "core.fsmonitor=false", "stash", "push"]));
}
