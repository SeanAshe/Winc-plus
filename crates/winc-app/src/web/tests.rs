//! `web` 的测试：联网失败的归类，以及错误输出里的用户名和密码不外露。

use super::*;

#[test]
fn curl_failures_are_sorted_into_reasons() {
    let curl = |code, stderr| Failure::from_curl(Some(code), stderr, false);
    let proxied = |code, stderr| Failure::from_curl(Some(code), stderr, true);
    assert_eq!(
        curl(56, "curl: (56) CONNECT tunnel failed, response 407"),
        Failure::ProxyAuth
    );
    assert_eq!(
        curl(
            56,
            "curl: (56) Received HTTP code 407 from proxy after CONNECT"
        ),
        Failure::ProxyAuth
    );
    assert_eq!(
        curl(5, "curl: (5) Could not resolve proxy: proxy.corp"),
        Failure::Proxy
    );
    assert_eq!(
        proxied(7, "curl: (7) Failed to connect to 10.0.0.1 port 8080"),
        Failure::Proxy
    );
    assert_eq!(
        proxied(56, "curl: (56) Proxy CONNECT aborted"),
        Failure::Proxy
    );
    assert_eq!(
        proxied(56, "curl: (56) Recv failure: Connection reset by peer"),
        Failure::Network
    );
    assert_eq!(
        curl(7, "curl: (7) Failed to connect to winc.si port 443"),
        Failure::Network
    );
    assert_eq!(
        curl(35, "curl: (35) schannel: CRYPT_E_NO_REVOCATION_CHECK"),
        Failure::Certificate
    );
    assert_eq!(curl(28, "curl: (28) Operation timed out"), Failure::Timeout);
    assert_eq!(curl(22, "curl: (22) error: 404"), Failure::Server);
    assert_eq!(
        curl(3, "\ncurl: (3) URL rejected\n"),
        Failure::Other("curl: (3) URL rejected".to_owned())
    );
    assert_eq!(
        Failure::from_curl(None, "", false),
        Failure::Other("exit ?".to_owned())
    );
}

#[test]
fn wget_failures_are_sorted_into_reasons() {
    assert_eq!(Failure::from_wget(Some(4), ""), Failure::Network);
    assert_eq!(Failure::from_wget(Some(5), ""), Failure::Certificate);
    assert_eq!(
        Failure::from_wget(Some(8), "Proxy tunneling failed: 407"),
        Failure::ProxyAuth
    );
    assert_eq!(Failure::from_wget(Some(8), ""), Failure::Server);
}

#[test]
fn credentials_in_addresses_are_hidden() {
    assert_eq!(
        redact("curl: (5) bad proxy http://u:p@proxy.corp:8080/ and https://a@b"),
        "curl: (5) bad proxy http://proxy.corp:8080/ and https://b"
    );
    assert_eq!(redact("mail me at a@b.c"), "mail me at a@b.c");
    assert_eq!(
        redact("see https://winc.si/x?a=b@c"),
        "see https://winc.si/x?a=b@c"
    );
    assert_eq!(
        Failure::from_curl(Some(3), "curl: (3) http://u:p@x", false),
        Failure::Other("curl: (3) http://x".to_owned())
    );
}

#[test]
fn curl_requests_addresses_literally() {
    let curl = curl();
    assert!(curl.get_args().any(|arg| arg == "--globoff"));
}
