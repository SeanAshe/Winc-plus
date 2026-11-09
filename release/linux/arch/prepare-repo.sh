#!/usr/bin/env bash
set -euo pipefail

fail() { printf '%s\n' "$*" >&2; exit 1; }

tag= out=
while (($#)); do
  case "$1" in
    --tag) tag=${2:?缺少 --tag 的值}; shift 2 ;;
    --out) out=${2:?缺少 --out 的值}; shift 2 ;;
    *) fail "未知参数 $1" ;;
  esac
done
[[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ && -n $out ]] ||
  fail '用法：bash prepare-repo.sh --tag v<稳定版版本> --out <新目录>'
[[ ! -e $out && ! -L $out ]] || fail "输出路径已存在 $out"

fingerprint=${WINC_ARCH_SIGNING_FINGERPRINT:-}
[[ $fingerprint =~ ^[0-9A-F]{40}$ ]] || fail '请设置 WINC_ARCH_SIGNING_FINGERPRINT 为 40 位大写十六进制公钥指纹'
github_repo=github.com/DEEIX-AI/Winc
repo=deeix-ai-winc
version=${tag#v}-1
package="winc-$version-x86_64.pkg.tar.zst"
base_url=https://winc.si/releases/arch/x86_64

parent=$(dirname -- "$out")
mkdir -p -- "$parent"
parent=$(cd -- "$parent" && pwd -P)
out="$parent/$(basename -- "$out")"
workdir=$(mktemp -d -- "$parent/.winc-arch.XXXXXXXX")
trap 'rm -rf -- "$workdir"' EXIT
stage="$workdir/arch"
mkdir -p -- "$stage/x86_64"

published=$(gh release view "$tag" --repo "$github_repo" --json isDraft,isPrerelease,publishedAt \
  --jq '(.isDraft == false and .isPrerelease == false and .publishedAt != null)')
[[ $published == true ]] || fail "$tag 必须是已正式发布的稳定版"
gh release download "$tag" --repo "$github_repo" --dir "$stage/x86_64" \
  --pattern "$package" --pattern SHA256SUMS

(
  cd "$stage/x86_64"
  hash=$(awk -v name="$package" '$2 == name { print $1 }' SHA256SUMS)
  [[ $hash =~ ^[0-9a-f]{64}$ ]] || fail "SHA256SUMS 缺少唯一且有效的 $package 校验值"
  printf '%s  %s\n' "$hash" "$package" | sha256sum --check --strict -
  metadata=$(bsdtar -xOf "$package" .PKGINFO)
  [[ $(awk -F ' = ' '$1 == "pkgname" {print $2}' <<< "$metadata") == winc &&
     $(awk -F ' = ' '$1 == "pkgver" {print $2}' <<< "$metadata") == "$version" &&
     $(awk -F ' = ' '$1 == "arch" {print $2}' <<< "$metadata") == x86_64 ]] ||
    fail "$package 的包名、版本或架构不符"

  gpg --batch --armor --export-options export-minimal --export "$fingerprint" > ../winc-arch.asc
  mkdir -m 0700 "$workdir/verify"
  verify_gpg=(gpg --no-options --homedir "$workdir/verify" --batch --no-auto-key-retrieve --no-auto-key-import)
  "${verify_gpg[@]}" --import ../winc-arch.asc
  exported=$("${verify_gpg[@]}" --with-colons --list-keys |
    awk -F: '$1 == "pub" {primary=1} $1 == "fpr" && primary {print $10; primary=0}')
  [[ $exported == "$fingerprint" ]] || fail '导出的公钥指纹不符'

  # 校验线上索引后再比较版本。只有索引及签名均返回 404 才视作首次部署。
  fetch_status() {
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 --silent --show-error --location \
      --connect-timeout 15 --max-time 60 --header 'Cache-Control: no-cache' \
      --output "$workdir/$1" --write-out '%{http_code}' "$base_url/$1"
  }
  db_status=$(fetch_status "$repo.db")
  sig_status=$(fetch_status "$repo.db.sig")
  case "$db_status:$sig_status" in
    404:404) ;;
    200:200)
      "${verify_gpg[@]}" --verify "$workdir/$repo.db.sig" "$workdir/$repo.db"
      desc=$(bsdtar -xOf "$workdir/$repo.db" --include '*/desc')
      live_version=$(awk '$0 == "%VERSION%" { getline; print }' <<< "$desc")
      [[ $live_version =~ ^[0-9]+\.[0-9]+\.[0-9]+-1$ ]] || fail '线上索引的版本无效'
      comparison=$(vercmp "$version" "$live_version")
      ((comparison >= 0)) || fail "线上版本 $live_version 更新，拒绝部署 $version"
      if ((comparison == 0)); then
        [[ $(awk '$0 == "%SHA256SUM%" { getline; print }' <<< "$desc") == "$hash" ]] ||
          fail "线上版本 $version 对应不同的包，拒绝替换"
      fi
      ;;
    *) fail "无法校验线上索引（HTTP $db_status / $sig_status）" ;;
  esac

  gpg --batch --no-armor --local-user "$fingerprint" --detach-sign "$package"
  repo-add --sign --key "$fingerprint" --include-sigs "$repo.db.tar.gz" "$package"
  "${verify_gpg[@]}" --verify "$package.sig" "$package"
  for index in db files; do
    "${verify_gpg[@]}" --verify "$repo.$index.tar.gz.sig" "$repo.$index.tar.gz"
    # 生成供静态网站直接上传的普通文件，避免部署工具跳过符号链接。
    rm -- "$repo.$index" "$repo.$index.sig"
    mv -- "$repo.$index.tar.gz" "$repo.$index"
    mv -- "$repo.$index.tar.gz.sig" "$repo.$index.sig"
  done
  rm -- SHA256SUMS
)

sed "s/@WINC_ARCH_SIGNING_FINGERPRINT@/$fingerprint/" \
  "$(dirname "$0")/install-arch.sh" > "$stage/install-arch.sh"
bash -n "$stage/install-arch.sh"
mv --no-clobber --no-target-directory -- "$stage" "$out"
[[ ! -e $stage ]] || fail "输出路径已存在 $out"
printf '已生成签名仓库 %s\n发布公钥指纹 %s\n' "$out" "$fingerprint"
