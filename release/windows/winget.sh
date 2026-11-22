#!/usr/bin/env bash
# Opens the pull request that adds a published version to microsoft/winget-pkgs, with Komac.
# WINGET_TOKEN is a classic token with the public_repo scope, from an account that owns a fork of
# winget-pkgs. Without it nothing is submitted, so the workflow is harmless until it is set up.
set -euo pipefail
tag="${1:?a release tag is required}"
version="${tag#v}"
identifier=DEEIX-AI.Winc
komac_version=2.17.2
komac_sha256=b03b020eec98acfb0b53e284a6e21815b884be33de6785b97e7a120f0e748966
if [[ -z "${WINGET_TOKEN:-}" ]]; then
  echo "::notice::WINGET_TOKEN is not set: $tag was not submitted to winget-pkgs"
  exit 0
fi
base="https://github.com/DEEIX-AI/Winc/releases/download/$tag"
workdir="$(mktemp -d)"
trap 'rm -rf -- "$workdir"' EXIT
curl -fsSL --retry 3 --output "$workdir/komac.tar.gz" \
  "https://github.com/russellbanks/Komac/releases/download/v$komac_version/komac-$komac_version-x86_64-unknown-linux-gnu.tar.gz"
echo "$komac_sha256  $workdir/komac.tar.gz" | sha256sum --check --quiet
tar -xzf "$workdir/komac.tar.gz" -C "$workdir" komac
# The installers only: the portable zip is not a winget package.
GITHUB_TOKEN="$WINGET_TOKEN" "$workdir/komac" update "$identifier" --version "$version" \
  --urls "$base/Winc-$version-windows-x64-setup.exe" "$base/Winc-$version-windows-x64.msi" --submit
