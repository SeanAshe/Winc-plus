#!/usr/bin/env bash
set -euo pipefail

fail() { printf '%s\n' "$*" >&2; exit 1; }

expected_fingerprint='@WINC_ARCH_SIGNING_FINGERPRINT@'
repo=deeix-ai-winc
base_url=https://winc.si/releases/arch
pacman_config=/etc/pacman.conf
repo_config=/etc/pacman.d/$repo.conf
include_line="Include = $repo_config"

(($# == 0)) || fail '用法：sudo bash install-arch.sh'
((EUID == 0)) || fail '请使用 sudo bash install-arch.sh'
[[ $(uname -m) == x86_64 ]] || fail 'Winc 的 Arch 软件源当前支持 x86_64'
. /etc/os-release
[[ ${ID:-} == arch || ${ID:-} == cachyos ]] || fail '此安装脚本适用于 Arch Linux 和 CachyOS'
[[ $expected_fingerprint =~ ^[0-9A-F]{40}$ ]] || fail '请从 Winc 官网下载正式安装脚本'
for command in curl gpg pacman pacman-key pacman-conf flock; do
  command -v "$command" >/dev/null || fail "缺少命令 $command"
done
architectures=$(pacman-conf --config "$pacman_config" Architecture)
server_url=$base_url/\$arch
if [[ $ID == cachyos ]]; then
  [[ -z $architectures ]] || grep -Fxq x86_64 <<< "$architectures" ||
    fail 'pacman 的 Architecture 必须允许 x86_64，请检查 /etc/pacman.conf'
  server_url=$base_url/x86_64
else
  [[ $architectures == x86_64 ]] ||
    fail 'pacman 的 Architecture 必须解析为 x86_64，请检查 /etc/pacman.conf'
fi

workdir=$(mktemp -d)
trap 'rm -rf -- "$workdir"' EXIT
download() {
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 --fail --silent --show-error --location \
    --connect-timeout 15 --max-time 60 --output "$workdir/${1##*/}" "$base_url/$1"
}
download winc-arch.asc
download "x86_64/$repo.db"
download "x86_64/$repo.db.sig"

mkdir -m 0700 "$workdir/verify"
verify_gpg=(gpg --no-options --homedir "$workdir/verify" --batch --no-auto-key-retrieve --no-auto-key-import)
fingerprints=$("${verify_gpg[@]}" --with-colons --show-keys "$workdir/winc-arch.asc" |
  awk -F: '$1 == "pub" {primary=1} $1 == "fpr" && primary {print $10; primary=0}')
[[ $fingerprints == "$expected_fingerprint" ]] || fail 'Winc 发布公钥指纹不符'
"${verify_gpg[@]}" --import "$workdir/winc-arch.asc"
"${verify_gpg[@]}" --verify "$workdir/$repo.db.sig" "$workdir/$repo.db"

# 从同一次配置检查到 pacman 完成，避免两个安装进程重复写入 Include。
exec 9>/run/lock/$repo-install.lock
flock -n 9 || fail '另一个 Winc 安装进程正在运行'
cat > "$workdir/repo.conf" <<EOF
[$repo]
SigLevel = Required TrustedOnly
Server = $server_url
EOF
[[ ! -L $repo_config ]] || fail "仓库配置是符号链接 $repo_config"
include_count=$(awk -v path="$repo_config" '
  /^[[:blank:]]*Include[[:blank:]]*=/ {
    sub(/^[^=]*=[[:blank:]]*/, "")
    sub(/[[:space:]]+$/, "")
    if ($0 == path) count++
  }
  END {print count+0}
' "$pacman_config")
if [[ -e $repo_config ]] && ! cmp -s "$repo_config" "$workdir/repo.conf"; then
  # CachyOS 上已启用的旧配置仍需通过下方的实际地址和签名设置检查。
  [[ $ID == cachyos && $include_count == 1 ]] &&
    cmp -s "$repo_config" <(sed 's|/x86_64$|/$arch|' "$workdir/repo.conf") ||
    fail "已有冲突的仓库配置 $repo_config"
fi
repos=$(pacman-conf --config "$pacman_config" --repo-list)
if grep -Fxq "$repo" <<< "$repos"; then
  server=$(pacman-conf --config "$pacman_config" --repo "$repo" Server)
  siglevel=$(pacman-conf --config "$pacman_config" --repo "$repo" SigLevel)
  usage=$(pacman-conf --config "$pacman_config" --repo "$repo" Usage)
  [[ $include_count == 1 && -f $repo_config && $server == "$base_url/x86_64" &&
     $siglevel == $'PackageRequired\nPackageTrustedOnly\nDatabaseRequired\nDatabaseTrustedOnly' && $usage == All ]] ||
    fail "已有冲突的 [$repo] 仓库配置"
elif [[ $include_count != 0 ]]; then
  fail "已有失效或重复的 $include_line"
fi

pacman-key --init
pacman-key --add "$workdir/winc-arch.asc"
pacman-key --lsign-key "$expected_fingerprint"
if [[ ! -e $repo_config ]]; then
  install -m0644 "$workdir/repo.conf" "$repo_config"
fi
if [[ $include_count == 0 ]]; then
  printf '\n%s\n' "$include_line" >> "$pacman_config"
fi
pacman -Syu --needed "$repo/winc"
