//! 连接目标的校验。

use super::*;

#[test]
fn hosts_aliases_and_users_are_accepted() {
    for text in ["devbox", "build-01.example.com", "alice@10.0.0.8", "a_b"] {
        assert_eq!(Target::new(text).unwrap().as_str(), text);
    }
}

#[test]
fn options_metacharacters_and_separators_are_rejected() {
    // 以 `-` 开头会被 `ssh` 当作选项；其余字符在 shell 或路径中有特殊含义。
    for text in [
        "",
        "-oProxyCommand=sh",
        "host name",
        "host;reboot",
        "host$(id)",
        "host/path",
        "host\\path",
        "[::1]",
        "主机",
    ] {
        assert_eq!(Target::new(text), None, "{text:?}");
    }
    assert!(Target::new(&"a".repeat(200)).is_some());
    assert!(Target::new(&"a".repeat(201)).is_none());
}

#[test]
fn a_port_may_follow_the_host() {
    let target = Target::new("root@10.0.0.8:22022").unwrap();
    assert_eq!(
        (target.host(), target.port()),
        ("root@10.0.0.8", Some(22022))
    );
    assert_eq!(target.as_str(), "root@10.0.0.8:22022");
    let plain = Target::new("devbox").unwrap();
    assert_eq!((plain.host(), plain.port()), ("devbox", None));
    assert_eq!(
        Target::with_port("devbox", Some(22)),
        Target::new("devbox:22")
    );
    assert_eq!(Target::with_port("devbox", None), Some(plain));
    // 端口只有一种写法；主机之后只能有一个端口。
    for text in [
        "host:",
        "host:0",
        "host:022",
        "host:+22",
        "host:65536",
        "host:22:23",
        "host:ssh",
        ":22",
        "-x:22",
    ] {
        assert_eq!(Target::new(text), None, "{text:?}");
    }
    assert_eq!(Target::with_port("host:22", None), None);
}
