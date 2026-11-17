//! 凭据存取规则的测试，使用进程内的表。

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

/// 返回用户名为 `username` 的凭据；密码在运行时生成，每次调用都不同。
fn credential(username: &str) -> Credential {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    Credential {
        username: username.to_owned(),
        password: format!("{}-{serial}", std::process::id()),
    }
}

/// 保存后按地址或按用户名都能取回；指定的用户名与保存的不同时没有结果。
#[test]
fn a_saved_credential_is_found_by_origin_and_by_username() {
    let origin = "https://find.credentials.test";
    let alice = credential("alice");
    assert!(find(origin, None).is_none());
    assert!(save(origin, &alice));
    assert!(find(origin, None).as_ref() == Some(&alice));
    assert!(find(origin, Some("alice")).as_ref() == Some(&alice));
    assert!(find(origin, Some("bob")).is_none());
}

/// 同一地址的第二个用户名不影响第一个；不指定用户名时取最近保存的。
#[test]
fn two_usernames_on_one_host_are_kept_apart() {
    let origin = "https://two.credentials.test";
    let (alice, bob) = (credential("alice"), credential("bob"));
    save(origin, &alice);
    save(origin, &bob);
    assert!(find(origin, Some("alice")).as_ref() == Some(&alice));
    assert!(find(origin, None).as_ref() == Some(&bob));
    // 删除 alice 的凭据：不指定用户名时仍取到 bob 的。
    forget(origin, Some("alice"));
    assert!(find(origin, Some("alice")).is_none());
    assert!(find(origin, None).as_ref() == Some(&bob));
    forget(origin, Some("bob"));
    assert!(find(origin, None).is_none());
}

/// 用户名含换行符时不保存；不是 `协议://主机` 的地址没有条目。
#[test]
fn malformed_input_is_not_stored() {
    assert!(!save("https://bad.credentials.test", &credential("a\nb")));
    assert!(find("https://bad.credentials.test", None).is_none());
    assert!(!save("bad.credentials.test", &credential("a")));
}

/// 同一地址的两个条目只计一次。
#[test]
fn hosts_are_counted_once() {
    let names = [
        "https://alice@count.test".to_owned(),
        "https://count.test".to_owned(),
        "http://other.test".to_owned(),
    ];
    assert_eq!(hosts(&names), 2);
}
