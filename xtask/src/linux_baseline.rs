//! Linux 安装包支持的最旧 glibc 的测试：各处声明的版本一致，构建用的镜像按摘要固定。

use std::path::Path;

fn release_file(name: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let path = root.join("release/linux").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `baseline.env` 中变量 `name` 的值。
fn setting(name: &str) -> String {
    let settings = release_file("baseline.env");
    let value = settings
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix('='));
    value.unwrap_or_else(|| panic!("缺少 {name}")).to_owned()
}

#[test]
fn every_package_declares_the_same_oldest_glibc() {
    let floor = setting("GLIBC_FLOOR");
    // deb 的依赖取自这个变量，Arch 的写在 PKGBUILD 里；rpm 的由 rpmbuild 从程序得出。
    assert!(release_file("package.sh").contains("libc6 (>= $GLIBC_FLOOR)"));
    assert!(release_file("arch/PKGBUILD").contains(&format!("'glibc>={floor}'")));
}

#[test]
fn the_build_images_have_that_glibc_and_are_pinned() {
    let floor = setting("GLIBC_FLOOR").replace('.', "_");
    for machine in ["x86_64", "aarch64"] {
        let image = setting(&format!("BASELINE_IMAGE_{machine}"));
        let (name, digest) = image.split_once("@sha256:").expect("按摘要固定");
        assert_eq!(name, format!("quay.io/pypa/manylinux_{floor}_{machine}"));
        assert!(digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
