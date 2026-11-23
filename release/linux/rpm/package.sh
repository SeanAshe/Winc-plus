#!/usr/bin/env bash
# RPM package built from the same Linux binaries as the deb, then installed into a clean Fedora and
# into AlmaLinux 8, whose glibc is the oldest supported one, to check that its dependencies resolve
# and the program runs.
set -euo pipefail
target="${1:?a Rust target triple is required}"
cd "$(dirname "$0")/../../.."
source release/linux/target.sh
version="$(node release/common/version.mjs)"
# RPM versions cannot contain `-`; `~` sorts prereleases before the final release, as in the deb.
rpm_version="${version//-/~}"
release_dir="target/$target/release"
workdir="$(mktemp -d)"
trap 'rm -rf -- "$workdir"' EXIT
files="$workdir/files"
mkdir -p "$files"
install -m755 "$release_dir/winc" "$release_dir/winc-pty" -t "$files"
install -m755 target/remote-server/winc-server-Linux-x86_64 \
  target/remote-server/winc-server-Linux-aarch64 -t "$files"
install -m644 release/linux/winc.desktop release/linux/winc-url-handler.desktop release/linux/winc.xml \
  crates/winc-app/assets/file-icons/project.svg "$release_dir/bundle/icons/winc.png" LICENSE NOTICE \
  -t "$files"
install -m644 release/linux/file-manager/* -t "$files"
install -m644 release/linux/rpm/winc.spec -t "$workdir"

# Fedora's base image has no rpmbuild; it is installed in a throwaway container that only packages
# the files above. The build tree stays in the container; only the RPM is handed back, owned by the
# runner's user.
image=fedora:42
package="Winc-$version-$platform.rpm"
docker run --rm --volume "$workdir:/work" "$image" bash -euc '
  dnf -y -q install rpm-build >/dev/null
  rpmbuild -bb --define "_topdir /tmp/rpmbuild" --define "winc_version $1" \
    --define "winc_files /work/files" /work/winc.spec
  built=(/tmp/rpmbuild/RPMS/"$4"/winc-*.rpm)
  ((${#built[@]} == 1)) || { echo "expected one RPM, found ${#built[@]}" >&2; exit 1; }
  cp "${built[0]}" "/work/$3"
  chown "$2" "/work/$3"
' bash "$rpm_version" "$(id -u):$(id -g)" "$package" "$machine"

for system in "$image" almalinux:8; do
  docker run --rm --volume "$workdir:/work:ro" "$system" bash -euc '
    dnf -y -q install "/work/$1" >/dev/null
    test "$(rpm -q --qf "%{VERSION}" winc)" = "$2"
    test "$(winc --version)" = "winc $3"
    test -x /usr/bin/winc-pty
    test -f /usr/lib/winc/winc-server-Linux-x86_64
    test -f /usr/lib/winc/winc-server-Linux-aarch64
    test -f /usr/share/kio/servicemenus/winc-open-folder.desktop
    grep -qx "MimeType=x-scheme-handler/winc;" /usr/share/applications/winc-url-handler.desktop
    test -f /usr/share/mime/packages/winc.xml
    test -f /usr/share/icons/hicolor/scalable/mimetypes/application-x-winc-project.svg
    test -f /usr/share/nemo/actions/winc-open-current-folder.nemo_action
    test -f /usr/share/licenses/winc/LICENSE
  ' bash "$package" "$rpm_version" "$version"
done

install -Dm644 "$workdir/$package" "$release_dir/bundle/rpm/$package"
