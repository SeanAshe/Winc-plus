#!/usr/bin/env bash
set -euo pipefail

target=${1:?缺少 Rust 目标平台}
[[ $target == x86_64-unknown-linux-gnu ]] || { echo "不支持的 Arch 打包目标 $target" >&2; exit 1; }
cd "$(dirname "$0")/../../.."
version=$(node release/common/version.mjs)
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "跳过非稳定版本 $version 的 Arch 打包"
  exit 0
fi

release_dir="target/$target/release"
workdir=$(mktemp -d)
trap 'rm -rf -- "$workdir"' EXIT
install -m755 "$release_dir/winc" "$release_dir/winc-pty" -t "$workdir"
install -m644 release/linux/arch/PKGBUILD release/linux/winc.desktop \
  "$release_dir/bundle/icons/winc.png" LICENSE NOTICE -t "$workdir"
install -m644 release/linux/file-manager/* -t "$workdir"

image=archlinux:base-devel
package="winc-$version-1-x86_64.pkg.tar.zst"
# 宿主机已经编译完毕。打包容器只封装文件，运行依赖由下方的安装验证检查。
docker run --rm --network none --user "$(id -u):$(id -g)" \
  --env HOME=/work --env "WINC_ARCH_VERSION=$version" \
  --volume "$workdir:/work" --workdir /work "$image" \
  makepkg --nodeps --nosign

docker run --rm --volume "$workdir:/work:ro" "$image" bash -euc '
  pacman -Syu --noconfirm
  pacman -U --noconfirm "/work/$1"
  test "$(pacman -Q winc)" = "winc $2-1"
  test "$(winc --version)" = "winc $2"
  test -x /usr/bin/winc-pty
  test -f /usr/share/kio/servicemenus/winc-open-folder.desktop
  test -f /usr/share/nemo/actions/winc-open-current-folder.nemo_action
' bash "$package" "$version"

install -Dm644 "$workdir/$package" "$release_dir/bundle/arch/$package"
