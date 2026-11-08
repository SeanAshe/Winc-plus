//! 应用内更新的测试：通道列表、appcast 的读取、版本比较，以及下载、核对和替换（用 `file://`
//! 地址，不连网）。

use ed25519_dalek::{Signer, SigningKey};

use super::*;

/// 测试用的签名密钥。
fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

/// 标准 base64 编码（测试中生成 appcast 用）。
fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(TABLE[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 一项 appcast 条目的 XML。
fn item_xml(version: &str, arch: &str, url: &str, length: usize, signature: &str) -> String {
    format!(
        "<item><title>{version}</title><sparkle:version>{version}</sparkle:version>\
         <sparkle:shortVersionString>{version}</sparkle:shortVersionString>\
         <enclosure url=\"{url}\" length=\"{length}\" type=\"application/octet-stream\" \
         sparkle:os=\"{os}\" arch=\"{arch}\" sparkle:edSignature=\"{signature}\"/></item>",
        os = platform::OS,
    )
}

/// `path` 的 `file://` 地址：Windows 上为 `file:///C:/…`。
pub(super) fn file_url(path: &Path) -> String {
    let path = path.display().to_string().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// 有没有 `curl`：下载类的测试要用它，没有时跳过这些测试。
pub(super) fn has_curl() -> bool {
    command("curl").arg("--version").output().is_ok()
}

/// 整个 appcast。
fn appcast(items: &[String]) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<rss version=\"2.0\" \
         xmlns:sparkle=\"{SPARKLE}\"><channel><title>winc</title>{}</channel></rss>\n",
        items.concat()
    )
}

/// 在 `feed` 末尾加上 Sparkle 格式的签名注释，用 `key` 签。
fn sign_feed(feed: &str, key: &SigningKey) -> String {
    let signature = encode_base64(&key.sign(feed.as_bytes()).to_bytes());
    format!(
        "{feed}<!-- sparkle-signatures:\nedSignature: {signature}\nlength: {}\n-->\n",
        feed.len()
    )
}

#[test]
fn channels_become_appcast_addresses() {
    assert_eq!(
        feeds("global=https://dl.example.com/winc/ cn=https://dl.example.cn/winc broken"),
        [
            format!("https://dl.example.com/winc/{FEED_FILE}"),
            format!("https://dl.example.cn/winc/{FEED_FILE}"),
        ]
    );
    assert!(feeds("").is_empty());
}

#[test]
fn base64_decodes_with_and_without_padding() {
    assert_eq!(decode_base64("aGk=").unwrap(), b"hi");
    assert_eq!(decode_base64("aGVsbG8gd29ybGQ").unwrap(), b"hello world");
    assert_eq!(decode_base64(" aG\nk= ").unwrap(), b"hi");
    assert!(decode_base64("a*b").is_none());
    for len in 0..8 {
        let bytes: Vec<u8> = (0..len).map(|i| i * 37 + 1).collect();
        assert_eq!(decode_base64(&encode_base64(&bytes)).unwrap(), bytes);
    }
    let key = signing_key().verifying_key();
    assert_eq!(public_key(&encode_base64(key.as_bytes())), Some(key));
    assert_eq!(public_key("aGk="), None);
}

#[test]
fn the_newest_item_for_this_architecture_wins() {
    let sig = encode_base64(&[1; 64]);
    let xml = appcast(&[
        item_xml("0.1.9", "x86_64", "https://x/9", 10, &sig),
        item_xml("0.1.10", "x86_64", "https://x/10", 20, &sig),
        item_xml("0.2.0", "aarch64", "https://x/a", 30, &sig),
    ]);
    let item = newest(&xml, "x86_64").unwrap();
    assert_eq!(
        (item.version.as_str(), item.url.as_str(), item.length),
        ("0.1.10", "https://x/10", 20)
    );
    assert_eq!(item.signature, [1; 64]);
    assert_eq!(newest(&xml, "aarch64").unwrap().version, "0.2.0");
    assert_eq!(newest(&xml, "riscv64"), None);
    // 签名长度不对、没有 `sparkle:os`、不是 XML：都不算。
    let short = appcast(&[item_xml(
        "1.0.0",
        "x86_64",
        "https://x",
        1,
        &encode_base64(&[1; 8]),
    )]);
    assert_eq!(newest(&short, "x86_64"), None);
    let mac = appcast(&[item_xml("1.0.0", "x86_64", "https://x", 1, &sig)
        .replace(&format!(" sparkle:os=\"{}\"", platform::OS), "")]);
    assert_eq!(newest(&mac, "x86_64"), None);
    assert_eq!(newest("not xml", "x86_64"), None);
}

#[test]
fn versions_compare_by_number() {
    assert!(newer("0.1.10", "0.1.9"));
    assert!(newer("1.0", "0.99.99"));
    assert!(newer("0.1.368.1", "0.1.368"));
    assert!(!newer("0.1.368", "0.1.368"));
    assert!(!newer("0.1.367", "0.1.368"));
    assert!(newer("1.2.3", "1.2.3-beta.10"));
    assert!(newer("1.2.3-beta.10", "1.2.3-beta.2"));
    assert!(!newer("1.2.3-beta.10", "1.2.3"));
    assert!(!newer("1.2.3+build.2", "1.2.3+build.1"));
}

#[test]
fn download_progress_is_a_whole_percent() {
    assert_eq!(percent(0, 200), 0);
    assert_eq!(percent(1, 200), 0);
    assert_eq!(percent(100, 200), 50);
    assert_eq!(percent(200, 200), 100);
    // 比预期多的字节（长度不对，核对时会报错）不超过 100；不知道长度时为 0。
    assert_eq!(percent(300, 200), 100);
    assert_eq!(percent(10, 0), 0);
}

#[test]
fn installation_type_selects_its_own_payload() {
    let sig = encode_base64(&[1; 64]);
    let tagged = |version, kind, url| {
        item_xml(version, "x86_64", url, 42, &sig)
            .replace(" arch=", &format!(" installer=\"{kind}\" arch="))
    };
    let xml = appcast(&[
        tagged("1.2.3-beta.4", "nsis", "https://x/nsis-beta"),
        tagged("1.2.3", "nsis", "https://x/nsis-stable"),
        tagged("1.2.3", "msi", "https://x/msi"),
        tagged("9.9.9", "unknown", "https://x/unknown"),
    ]);
    assert_eq!(
        newest_for_installer(&xml, "x86_64", Some("nsis"))
            .unwrap()
            .url,
        "https://x/nsis-stable"
    );
    assert_eq!(
        newest_for_installer(&xml, "x86_64", Some("msi"))
            .unwrap()
            .url,
        "https://x/msi"
    );
    assert!(newest_for_installer(&xml, "aarch64", Some("msi")).is_none());
    assert!(newest_for_installer(&xml, "x86_64", Some("inno")).is_none());
}

/// 发布构建后，以实际产物交叉核对 Node 签名、appcast 解析和 Rust 验证。
#[test]
#[ignore = "requires signed release packages and the build's update public key"]
fn release_appcast_matches_packages() {
    let target = std::env::var("TARGET").expect("TARGET is set by the native release workflow");
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(target)
        .join("release/bundle");
    let Some(encoded) = option_env!("WINC_UPDATE_PUBLIC_KEY") else {
        panic!("release update public key");
    };
    let key = public_key(encoded).expect("valid release update public key");
    let bytes = std::fs::read(bundle.join("updates").join(FEED_FILE)).unwrap();
    let xml =
        std::str::from_utf8(signed_part(&bytes, &key).expect("signed release appcast")).unwrap();
    #[cfg(windows)]
    let packages = [
        (Some("nsis"), "nsis", "windows-x64-setup.exe"),
        (Some("msi"), "msi", "windows-x64.msi"),
    ];
    #[cfg(not(windows))]
    let packages = [(None, "appimage", "linux-x64.AppImage")];
    for (installer, directory, suffix) in packages {
        let item = newest_for_installer(xml, std::env::consts::ARCH, installer).unwrap();
        assert_eq!(item.version, crate::release_version::update_version());
        let name = format!("Winc-{}-{suffix}", env!("CARGO_PKG_VERSION"));
        let payload = std::fs::read(bundle.join(directory).join(name)).unwrap();
        verify(&item, &payload, &key)
            .expect("release payload signature matches the embedded public key");
    }
}

/// 下载签名正确的文件替换原来的 AppImage，保留可执行；签名或大小不对时原文件不动，也不留下
/// 下载到一半的文件。
#[cfg(unix)]
#[test]
fn a_verified_download_replaces_the_appimage() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("winc-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let appimage = dir.join("winc.AppImage");
    std::fs::write(&appimage, b"old version").unwrap();
    std::fs::set_permissions(&appimage, std::fs::Permissions::from_mode(0o700)).unwrap();
    let new = dir.join("served.bin");
    let bytes = b"new version, a bit longer".to_vec();
    std::fs::write(&new, &bytes).unwrap();
    let key = signing_key();
    let item = Item {
        version: "9.9.9".into(),
        url: file_url(&new),
        length: bytes.len() as u64,
        signature: key.sign(&bytes).to_bytes(),
    };
    let leftovers = || {
        std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".update")
            })
            .count()
    };

    // 别的密钥签的、长度不对的：报错，原文件不动。
    let other = SigningKey::from_bytes(&[9; 32]).verifying_key();
    let error = appimage::prepare(&item, &appimage, &other, &|_| {}).unwrap_err();
    assert!(error.contains("签名"), "{error}");
    let long = Item {
        length: 999,
        ..item.clone()
    };
    assert!(appimage::prepare(&long, &appimage, &key.verifying_key(), &|_| {}).is_err());
    let missing = Item {
        url: file_url(&dir.join("missing")),
        ..item.clone()
    };
    assert!(appimage::prepare(&missing, &appimage, &key.verifying_key(), &|_| {}).is_err());
    assert_eq!(std::fs::read(&appimage).unwrap(), b"old version");
    assert_eq!(leftovers(), 0);

    // 下载期间报告的百分比不超过 100，也不倒退。
    let reported = std::cell::RefCell::new(Vec::new());
    let progress = |percent: u8| reported.borrow_mut().push(percent);
    let program = appimage::prepare(&item, &appimage, &key.verifying_key(), &progress).unwrap();
    let reported = reported.into_inner();
    assert!(reported.windows(2).all(|w| w[0] <= w[1]), "{reported:?}");
    assert!(reported.iter().all(|&p| p <= 100), "{reported:?}");
    assert_eq!(program, appimage);
    assert_eq!(std::fs::read(&appimage).unwrap(), bytes);
    let mode = std::fs::metadata(&appimage).unwrap().permissions().mode();
    assert_eq!(mode & 0o755, 0o755, "{mode:o}");
    assert_eq!(leftovers(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// appcast 的签名：签的是注释之前的内容；改动内容、换了密钥、没有签名都不认。
#[test]
fn feeds_must_carry_a_matching_signature() {
    let key = signing_key();
    let feed = appcast(&[item_xml(
        "1.0.0",
        "x86_64",
        "https://x",
        1,
        &encode_base64(&[1; 64]),
    )]);
    let signed = sign_feed(&feed, &key);
    let part = signed_part(signed.as_bytes(), &key.verifying_key()).unwrap();
    assert_eq!(part, feed.as_bytes());
    let tampered = signed.replace("1.0.0", "9.0.0");
    assert_eq!(signed_part(tampered.as_bytes(), &key.verifying_key()), None);
    let other = SigningKey::from_bytes(&[9; 32]).verifying_key();
    assert_eq!(signed_part(signed.as_bytes(), &other), None);
    assert_eq!(signed_part(feed.as_bytes(), &key.verifying_key()), None);
}

/// 请求多个通道时，连不上的、内容不对的、没签名的都不影响结果，取到第一个签名对得上的 appcast。
#[test]
fn the_first_verified_feed_wins() {
    if !has_curl() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("winc-feeds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let key = signing_key();
    let good = dir.join("good.xml");
    std::fs::write(&good, sign_feed(&appcast(&[]), &key)).unwrap();
    let unsigned = dir.join("unsigned.xml");
    std::fs::write(&unsigned, appcast(&[])).unwrap();
    let broken = dir.join("broken.xml");
    std::fs::write(&broken, sign_feed("<html>maintenance</html>", &key)).unwrap();
    let url = file_url;
    let feeds = [
        url(&dir.join("missing.xml")),
        url(&unsigned),
        url(&broken),
        url(&good),
    ];
    let verifying = key.verifying_key();
    assert!(race(&feeds, &verifying).unwrap().contains("<channel>"));
    assert_eq!(race(&feeds[..3], &verifying), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 生成本机试用的更新源：把 `WINC_DEMO_PAYLOAD` 作为版本 9.9.9 放进 `WINC_DEMO_FEED` 目录，配上
/// 用测试密钥签好的 appcast，并打印构建时要设的 `WINC_UPDATE_PUBLIC_KEY`。
#[test]
#[ignore = "生成试用的更新源，手动运行"]
fn demo_feed() {
    let dir = PathBuf::from(std::env::var_os("WINC_DEMO_FEED").expect("WINC_DEMO_FEED"));
    let payload =
        std::fs::read(std::env::var_os("WINC_DEMO_PAYLOAD").expect("WINC_DEMO_PAYLOAD")).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let key = signing_key();
    let arch = std::env::consts::ARCH;
    let name = format!("winc-9.9.9-{arch}.AppImage");
    std::fs::write(dir.join(&name), &payload).unwrap();
    let signature = encode_base64(&key.sign(&payload).to_bytes());
    let url = file_url(&dir.join(&name));
    let feed = appcast(&[item_xml("9.9.9", arch, &url, payload.len(), &signature)]);
    std::fs::write(dir.join(FEED_FILE), sign_feed(&feed, &key)).unwrap();
    println!(
        "WINC_UPDATE_PUBLIC_KEY={}",
        encode_base64(key.verifying_key().as_bytes())
    );
}
