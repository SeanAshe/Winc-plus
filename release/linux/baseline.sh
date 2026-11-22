#!/usr/bin/env bash
# Runs a command in the build image with the oldest supported C library. What is linked there needs
# no newer glibc than GLIBC_FLOOR; verify-baseline.sh checks the result. Every `cargo` of the Linux
# release goes through here, so no build script or macro is ever linked against the runner's glibc.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=release/linux/baseline.env
source "$here/baseline.env"
machine="$(uname -m)"
image_name="BASELINE_IMAGE_$machine"
image="${!image_name:?no build image for $machine}"
# The toolchain installed on the runner is used as it is, mounted at the same paths.
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
# Only what the build reads is passed on; signing keys stay outside.
passed=()
for name in CARGO_INCREMENTAL RUSTUP_TOOLCHAIN TARGET WINC_UPDATE_CHANNELS WINC_UPDATE_PUBLIC_KEY; do
  [[ -n "${!name+set}" ]] && passed+=(--env "$name")
done
exec docker run --rm --user "$(id -u):$(id -g)" \
  --volume "$root:$root" --volume "$cargo_home:$cargo_home" --volume "$rustup_home:$rustup_home" \
  --workdir "$root" --env HOME="$HOME" --env CARGO_HOME="$cargo_home" --env RUSTUP_HOME="$rustup_home" \
  ${passed[@]+"${passed[@]}"} "$image" \
  bash -c 'PATH="$CARGO_HOME/bin:$PATH" exec "$@"' bash "$@"
