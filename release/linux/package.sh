#!/usr/bin/env bash
# Native Rust packages: an AppImage with bundled libraries, and a system-managed deb.
set -euo pipefail
target="${1:?a Rust target triple is required}"
[[ "$target" == x86_64-unknown-linux-gnu ]] || { echo "unsupported Linux target: $target" >&2; exit 1; }
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
version="$(node release/common/version.mjs)"
deb_version="${version/-/~}"
release="$root/target/$target/release"
bundle="$release/bundle"
tools="$root/target/release-tools"
stem="Winc-$version-linux-x64"
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
appdir="$workdir/winc.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps"
install -m755 "$release/winc" "$release/winc-pty" "$appdir/usr/bin/"
install -m644 "$bundle/icons/winc.png" "$appdir/usr/share/icons/hicolor/256x256/apps/winc.png"
install -m644 release/linux/winc.desktop "$appdir/usr/share/applications/winc.desktop"
desktop-file-validate "$appdir/usr/share/applications/winc.desktop"

# Keep the deb independent of AppImage's patched RPATHs and bundled libraries.
deb="$workdir/deb"
mkdir -p "$deb/DEBIAN" "$deb/usr/share/doc/winc" "$bundle/deb" "$bundle/appimage"
cp -a "$appdir/usr" "$deb/"
# The remote server is uploaded to Linux hosts, never run from here: it lives beside the libraries,
# not on the PATH. Both architectures ship in every package.
servers=("$root/target/remote-server/winc-server-Linux-x86_64" "$root/target/remote-server/winc-server-Linux-aarch64")
install -Dm755 "${servers[@]}" -t "$deb/usr/lib/winc"
# File manager context menus belong to the installed package only: the AppImage installs nothing.
install -Dm644 release/linux/file-manager/winc-open-file.desktop release/linux/file-manager/winc-open-folder.desktop \
  -t "$deb/usr/share/kio/servicemenus"
install -Dm644 release/linux/file-manager/*.nemo_action -t "$deb/usr/share/nemo/actions"
install -m644 LICENSE NOTICE "$deb/usr/share/doc/winc/"
cat > "$deb/DEBIAN/control" <<EOF
Package: winc
Version: $deb_version
Section: editors
Priority: optional
Architecture: amd64
Maintainer: DEEIX-AI <41963680+Noxiven@users.noreply.github.com>
Homepage: https://winc.si
Depends: libc6 (>= 2.39), libgcc-s1, fontconfig, fonts-dejavu-core, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libx11-xcb1, libxcb1, libxcursor1, libxi6, libxrandr2, libwayland-client0
Description: A lighter, faster desktop code editor
 A native Rust editor with its own rendering engine, Git integration and terminal.
EOF
dpkg-deb --root-owner-group --build "$deb" "$bundle/deb/$stem.deb"

APPIMAGE_EXTRACT_AND_RUN=1 "$tools/linuxdeploy.AppImage" --appimage-extract-and-run \
  --appdir "$appdir" --executable "$appdir/usr/bin/winc" --executable "$appdir/usr/bin/winc-pty" \
  --desktop-file "$appdir/usr/share/applications/winc.desktop" \
  --icon-file "$appdir/usr/share/icons/hicolor/256x256/apps/winc.png" \
  --exclude-library 'libwayland-*' 2>&1 | tee "${LINUXDEPLOY_DEBUG_LOG:-$workdir/linuxdeploy.log}"
# Added after linuxdeploy: it would otherwise treat the static servers as libraries to patch.
install -Dm755 "${servers[@]}" -t "$appdir/usr/lib/winc"
rm -f "$appdir/AppRun"
install -m755 release/linux/AppRun "$appdir/AppRun"
# The AppDir holds servers for two architectures, so appimagetool cannot infer the architecture of
# the AppImage from the files in it and needs it named.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$tools/appimagetool.AppImage" --appimage-extract-and-run \
  --runtime-file "$tools/runtime-x86_64" "$appdir" "$bundle/appimage/$stem.AppImage"
