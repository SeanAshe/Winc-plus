# shellcheck shell=bash disable=SC2034
# Names of one Linux target in each packaging system. Sourced with `target` set to the Rust triple.
case "${target:?a Rust target triple is required}" in
  x86_64-unknown-linux-gnu) platform=linux-x64 machine=x86_64 deb_arch=amd64 ;;
  aarch64-unknown-linux-gnu) platform=linux-arm64 machine=aarch64 deb_arch=arm64 ;;
  *) echo "unsupported Linux target: $target" >&2; exit 1 ;;
esac
