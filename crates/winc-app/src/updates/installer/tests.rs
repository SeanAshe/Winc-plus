//! `installer` 的测试：区分安装版与便携版的标记，校验通过的安装包保留到退出时使用。

use ed25519_dalek::{Signer, SigningKey};

use super::super::tests::{file_url, has_curl};
use super::*;

#[test]
fn markers_distinguish_installed_and_portable_copies() {
    let dir = std::env::temp_dir().join(format!("winc-installation-kind-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("winc.exe");
    let marker = dir.join("winc-installation");
    let _ = std::fs::remove_file(&marker);
    assert_eq!(kind_at(&exe), None);
    for (text, expected) in [
        ("nsis\n", Some(Kind::Nsis)),
        ("msi\r\n", Some(Kind::Msi)),
        ("unknown", None),
    ] {
        std::fs::write(&marker, text).unwrap();
        assert_eq!(kind_at(&exe), expected);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// 验证通过后安装包留在目录中，签名不对时删除整个目录。
///
/// 没有 `curl` 时（下载命令不可用）跳过这个测试。
#[test]
fn a_verified_installer_is_kept_for_exit() {
    if !has_curl() {
        return;
    }
    let root = std::env::temp_dir().join(format!("winc-installer-{}", std::process::id()));
    let dir = root.join("update");
    std::fs::create_dir_all(&root).unwrap();
    let served = root.join("served.exe");
    let bytes = b"new installer".to_vec();
    std::fs::write(&served, &bytes).unwrap();
    let key = SigningKey::from_bytes(&[7; 32]);
    let item = Item {
        version: "9.9.9".into(),
        url: file_url(&served),
        length: bytes.len() as u64,
        signature: key.sign(&bytes).to_bytes(),
    };

    let other = SigningKey::from_bytes(&[9; 32]).verifying_key();
    assert!(prepare_in(&dir, &item, &other, Kind::Nsis, &|_| {}).is_err());
    assert!(!dir.exists());

    let setup = prepare_in(&dir, &item, &key.verifying_key(), Kind::Nsis, &|_| {}).unwrap();
    assert_eq!(std::fs::read(&setup).unwrap(), bytes);
    assert_eq!(setup.extension().unwrap(), "exe");
    let setup = prepare_in(&dir, &item, &key.verifying_key(), Kind::Msi, &|_| {}).unwrap();
    assert_eq!(setup.extension().unwrap(), "msi");
    assert_eq!(std::fs::read(&setup).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(&root);
}
